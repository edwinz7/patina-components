//! USB Bus driver binding implementation.
//!
//! The [`UsbBusDriver`] implements [`patina::driver_binding::DriverBinding`] to
//! manage the USB Bus. It produces USB IO.
//!
//! ## License
//!
//! Copyright (c) Microsoft Corporation.
//!
//! SPDX-License-Identifier: Apache-2.0
//!

// possibly upstream these to r_efi
#[path = "../../protocols/device_path.rs"]
mod device_path;

use alloc::boxed::Box;
//use core::{ffi::c_void, ptr{self, NonNull}};
use core::{
    mem::MaybeUninit,
    ptr::{self, NonNull},
};

//use device_path::EfiDevPathPtr;
//use r_efi::{efi, efi::protocols::usb_io, protocols::device_path::Protocol as EfiDevicePathProtocol};
use crate::usb_2_host_controller::{self, Protocol as Usb2HcProtocol};
use r_efi::{efi, protocols::device_path::Protocol as EfiDevicePathProtocol};

use patina::{
    pi::{
        list_entry,
        protocol::status_code,
        status_code::{EFI_IO_BUS_USB, EFI_IOB_PC_INIT, EFI_PROGRESS_CODE},
    },
    uefi::{boot_services::BootServices, driver_binding::DriverBinding},
};

use crate::usb_bus_defs::{
    EfiUsbBusProtocol, USB_BUS_SIGNATURE, USB_INTERFACE_SIGNATURE, USB_MAX_DEVICES, UsbBus, UsbDevice, UsbInterface,
};

//use patina::vendor_protocols::hid_io;
//use crate::{control_transfers, descriptors, device::UsbBusDevice, hid_io_impl, interrupt_transfers, usb_hid_defs::*};

/// USB Bus driver that implements [`DriverBinding`].
pub struct UsbBusDriver {
    agent: efi::Handle,
}

fn usb_bus_build_protocol<U: BootServices + 'static>(
    boot_services: &'static U,
    agent: efi::Handle,
    controller: efi::Handle,
    _remaining_device_path: Option<NonNull<EfiDevicePathProtocol>>,
) -> Result<(), efi::Status> {
    let device_path = unsafe {
        boot_services.open_protocol::<EfiDevicePathProtocol>(
            controller,
            agent,
            controller,
            efi::OPEN_PROTOCOL_BY_DRIVER,
        )?
    };

    let usb2_hc = match unsafe {
        boot_services.open_protocol::<Usb2HcProtocol>(controller, agent, controller, efi::OPEN_PROTOCOL_BY_DRIVER)
    } {
        Ok(usb2_hc) => usb2_hc,
        Err(status) => {
            boot_services.close_protocol(controller, &device_path::PROTOCOL_GUID, agent, controller).ok();
            return Err(status);
        }
    };

    let mut bus = Box::new(UsbBus {
        signature: USB_BUS_SIGNATURE as usize,
        bus_id: EfiUsbBusProtocol { reserved: 0 },
        host_handle: controller,
        device_path: device_path as *mut _,
        usb2_hc: usb2_hc as *mut Usb2HcProtocol as *mut _,
        max_devices: USB_MAX_DEVICES as u32,
        devices: [ptr::null_mut(); 256],
        wanted_usb_io_dp_list: list_entry::Entry { forward_link: ptr::null_mut(), back_link: ptr::null_mut() },
    });

    if usb2_hc.major_revision == 0x3 {
        bus.max_devices = 256;
    }

    let mut root_hub = Box::new(UsbDevice {
        bus: ptr::null_mut(),
        speed: 0,
        address: 0,
        max_packet0: 0,
        dev_desc: ptr::null_mut(),
        active_config: ptr::null_mut(),
        lang_id: [0; 16],
        total_lang_id: 0,
        num_of_interface: 1,
        interfaces: [ptr::null_mut(); 16],
        translator: ptr::null_mut(),
        parent_addr: 0,
        parent_if: ptr::null_mut(),
        parent_port: 0,
        tier: 0,
        connected: false.into(),
        disconnect_fail: false.into(),
    });
    let root_if = Box::new(MaybeUninit::<UsbInterface>::uninit());

    let bus_ptr = &mut *bus as *mut UsbBus;
    let root_hub_ptr = &mut *root_hub as *mut UsbDevice;
    let root_if_ptr = root_if.as_ptr() as *mut UsbInterface;
    let list_head = &mut bus.wanted_usb_io_dp_list as *mut list_entry::Entry;
    bus.wanted_usb_io_dp_list.forward_link = list_head;
    bus.wanted_usb_io_dp_list.back_link = list_head;
    root_hub.bus = bus_ptr;
    root_hub.interfaces[0] = root_if_ptr;
    unsafe {
        ptr::addr_of_mut!((*root_if_ptr).signature).write(USB_INTERFACE_SIGNATURE as usize);
        ptr::addr_of_mut!((*root_if_ptr).device).write(root_hub_ptr);
        ptr::addr_of_mut!((*root_if_ptr).device_path).write(device_path as *mut _);
    }
    bus.devices[0] = root_hub_ptr;

    if let Err(status) =
        boot_services.install_protocol_interface(Some(controller), Box::new(EfiUsbBusProtocol { reserved: 0 }))
    {
        boot_services.close_protocol(controller, &usb_2_host_controller::PROTOCOL_GUID, agent, controller).ok();
        boot_services.close_protocol(controller, &device_path::PROTOCOL_GUID, agent, controller).ok();
        return Err(status);
    }

    Box::leak(bus);
    Box::leak(root_hub);
    Box::leak(root_if);
    Ok(())
}

impl UsbBusDriver {
    /// Creates a new USB Bus driver bound to the given agent handle.
    pub fn new(agent: efi::Handle) -> Self {
        Self { agent }
    }
}

// efi::Handle is an opaque *mut c_void that is never actually dereferenced as a pointer.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
impl DriverBinding for UsbBusDriver {
    /// Tests if the given controller has USB IO with HID interface class.
    #[coverage(off)]
    fn driver_binding_supported<U: BootServices + 'static>(
        &self,
        boot_services: &'static U,
        controller: efi::Handle,
        remaining_device_path: Option<NonNull<EfiDevicePathProtocol>>,
    ) -> Result<bool, efi::Status> {
        // SAFETY: Usb2HcProtocol layout matches the USB 2.0 Host Controller GUID.

        if let Some(remaining_device_path) = remaining_device_path {
            let remaining_device_path = remaining_device_path.as_ptr();
            let is_end_device_path = unsafe {
                (*remaining_device_path).r#type == device_path::TYPE_END
                    && (*remaining_device_path).sub_type == device_path::END_ENTIRE_DEVICE_PATH_SUBTYPE
            };
            if !is_end_device_path {
                let device_path_node = unsafe { &*remaining_device_path };
                if device_path_node.r#type != device_path::TYPE_MESSAGING
                    || (device_path_node.sub_type != device_path::MSG_USB_DP
                        && device_path_node.sub_type != device_path::MSG_USB_CLASS_DP
                        && device_path_node.sub_type != device_path::MSG_USB_WWID_DP)
                {
                    return Ok(false);
                }
            }
        }

        if let Err(status) = unsafe {
            boot_services.open_protocol::<Usb2HcProtocol>(
                controller,
                self.agent,
                controller,
                efi::OPEN_PROTOCOL_BY_DRIVER,
            )
        } {
            if status == efi::Status::ALREADY_STARTED {
                return Ok(true);
            } else {
                return Err(status);
            }
        };

        boot_services.close_protocol(controller, &usb_2_host_controller::PROTOCOL_GUID, self.agent, controller).ok();

        if let Err(status) = unsafe {
            boot_services.open_protocol::<device_path::Protocol>(
                controller,
                self.agent,
                controller,
                efi::OPEN_PROTOCOL_BY_DRIVER,
            )
        } {
            if status == efi::Status::ALREADY_STARTED {
                return Ok(true);
            } else {
                return Err(status);
            }
        };

        boot_services.close_protocol(controller, &device_path::PROTOCOL_GUID, self.agent, controller).ok();

        Ok(true)
    }

    /// Starts USB Bus support for the given controller.
    fn driver_binding_start<U: BootServices + 'static>(
        &mut self,
        boot_services: &'static U,
        controller: efi::Handle,
        remaining_device_path: Option<NonNull<EfiDevicePathProtocol>>,
    ) -> Result<(), efi::Status> {
        log::trace!("USB Bus: driver_binding_start on controller {:?}", controller);

        // The C driver obtains the parent device path before initializing the bus.
        // The Rust implementation currently has no status-code reporting service,
        // so the protocol lookup is retained while its value is intentionally unused.
        let _parent_device_path = unsafe {
            boot_services.open_protocol::<device_path::Protocol>(
                controller,
                self.agent,
                controller,
                efi::OPEN_PROTOCOL_GET_PROTOCOL,
            )?
        };

        // SAFETY: `p` is the only mutable reference to the `StatusCodeRuntimeProtocol` in this scope.
        let Ok(p) = (unsafe { boot_services.locate_protocol::<status_code::StatusCodeProtocol>(None) }) else {
            log::error!("Performance: Fail to find status code protocol.");
            return Err(efi::Status::NOT_FOUND);
        };

        let _status = p.report_status_code(
            EFI_PROGRESS_CODE,
            EFI_IO_BUS_USB | EFI_IOB_PC_INIT,
            0,
            patina::guid::CALLER_ID.as_efi_guid(),
        )?;
        // report_status_code

        let bus_protocol_exists = unsafe {
            boot_services
                .open_protocol::<EfiUsbBusProtocol>(controller, self.agent, controller, efi::OPEN_PROTOCOL_GET_PROTOCOL)
                .is_ok()
        };

        if !bus_protocol_exists {
            // first start
        } else {
            //
            // USB bus driver needs to control the recursive connect policy of the bus, only those wanted
            // USB child devices will be recursively connected.
            // remaining_device_path indicates the child USB device which users want to fully recursively connect this time.
            // All wanted USB child devices will be remembered by the USB bus driver itself.
            // If remaining_device_path is NULL, all the USB child devices in the USB bus are wanted devices.
            //
            // Save the passed in RemainingDevicePath this time
            //
            if remaining_device_path.is_some_and(|path| unsafe {
                (*path.as_ptr()).r#type == device_path::TYPE_END
                    && (*path.as_ptr()).sub_type == device_path::END_ENTIRE_DEVICE_PATH_SUBTYPE
            }) {
                return Ok(());
            }

            // implement UsbBusAddWantedUsbIoDP

            // implement UsbBusRecursivelyConnectWantedUsbIo

            // Wanted-device-path storage and recursive child connection are not
            // implemented in the current Rust bus model yet.
            log::debug!("USB Bus: existing bus requires child connection handling");
            return Ok(());
        }

        usb_bus_build_protocol(boot_services, self.agent, controller, remaining_device_path)?;

        Ok(())
    }

    /// Stops USB Bus support for the given controller.
    fn driver_binding_stop<U: BootServices + 'static>(
        &mut self,
        _boot_services: &'static U,
        controller: efi::Handle,
        _number_of_children: usize,
        _child_handle_buffer: Option<NonNull<efi::Handle>>,
    ) -> Result<(), efi::Status> {
        log::trace!("USB Bus: driver_binding_stop on controller {:?}", controller);

        Ok(())
    }
}
/*
impl UsbBusDriver {
    fn close_usb_io(&self, boot_services: &impl BootServices, controller: efi::Handle) {
        if let Err(status) = boot_services.close_protocol(controller, &usb_io::PROTOCOL_GUID, self.agent, controller) {
            log::error!("USB Bus: error closing USB IO protocol: {status:x?}");
        }
    }
}
*/
