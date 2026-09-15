//! USB HID Driver — produces the HidIo protocol on USB HID devices.
//!
//! This crate implements a UEFI Driver Binding that consumes the USB IO protocol
//! and produces the HidIo protocol for each USB HID device it manages.
//!
//! ## License
//!
//! Copyright (c) Microsoft Corporation.
//!
//! SPDX-License-Identifier: Apache-2.0
//!
#![cfg_attr(not(test), no_std)]
#![feature(coverage_attribute)]

extern crate alloc;

#[path = "../../protocols/usb_2_host_controller.rs"]
pub(crate) mod usb_2_host_controller;
#[path = "../../protocols/device_path_temp.rs"]
pub(crate) mod device_path_temp;
pub(crate) mod usb_bus_defs;
pub(crate) mod usb_desc;
pub(crate) mod usb_enumer;
pub(crate) mod usb_hub;
pub(crate) mod usb_io_impl;
pub(crate) mod usb_utility;

//#[cfg(test)]
//pub(crate) mod test_stubs;

use alloc::{boxed::Box, vec::Vec};
use core::mem::MaybeUninit;
use core::ptr;
use r_efi::efi;

use patina::{
    component::{
        component,
        service::{
            Service,
            uefi_services::{
                driver_model::{driver::DriverServices, driver_binding::DriverBinding},
                protocol::{Handle, OpenAttributes, ProtocolError, ProtocolPtr, ProtocolServices, ProtocolServicesExt},
            },
        },
    },
    error::{Result, EfiError},
    pi::{
        protocol::status_code,
        status_code::{EFI_IO_BUS_USB, EFI_IOB_PC_INIT, EFI_PROGRESS_CODE},
    },
    uefi::device_path::walker::DevicePathWalker,
    protocol::ProtocolInterface,
};

use crate::usb_2_host_controller::Protocol as Usb2HcProtocol;
use crate::usb_bus_defs::{
    EfiUsbBusProtocol, USB_BUS_SIGNATURE, USB_INTERFACE_SIGNATURE, USB_MAX_DEVICES, UsbBus, UsbDevice,
    UsbDevicePathList, UsbInterface,
};
use crate::usb_utility::{usb_bus_add_wanted_usb_io_dp, usb_bus_recursively_connect_wanted_usb_io};

pub struct UsbBusDriver {
    protocols: Service<dyn ProtocolServices>,
    drivers: Service<dyn DriverServices>,
}

impl DriverBinding for UsbBusDriver {
    /// Tests if the given controller has USB IO with HID interface class.
    #[coverage(off)]
    fn supported(
        &self,
        agent: Handle,
        controller: Handle,
        remaining_device_path: Option<DevicePathWalker>,
    ) -> core::result::Result<(), ProtocolError> {
        // SAFETY: Usb2HcProtocol layout matches the USB 2.0 Host Controller GUID.

        if let Some(mut remaining_device_path) = remaining_device_path {
            let device_path_node = remaining_device_path.next().ok_or(ProtocolError::InvalidParameter)?;
            let device_path_header = device_path_node.header();
            let is_end_device_path = device_path_header.r#type == device_path_temp::TYPE_END
                && device_path_header.sub_type == device_path_temp::END_ENTIRE_DEVICE_PATH_SUBTYPE;
            if !is_end_device_path {
                if device_path_header.r#type != device_path_temp::TYPE_MESSAGING
                    || (device_path_header.sub_type != device_path_temp::MSG_USB_DP
                        && device_path_header.sub_type != device_path_temp::MSG_USB_CLASS_DP
                        && device_path_header.sub_type != device_path_temp::MSG_USB_WWID_DP)
                {
                    return Err(ProtocolError::from(EfiError::Unsupported));
                }
            }
        }

        if let Err(status) = self.protocols.open_interface(
            controller,
            Usb2HcProtocol::PROTOCOL_GUID,
            agent,
            OpenAttributes::ByDriver { controller },
        ) {
            if status == ProtocolError::AlreadyStarted {
                return Ok(());
            } else {
                return Err(status);
            }
        };

        self.protocols
            .close_interface(controller, Usb2HcProtocol::PROTOCOL_GUID, agent, Some(controller))
            .ok();

        if let Err(status) = self.protocols.open_interface(
            controller,
            device_path_temp::Protocol::PROTOCOL_GUID,
            agent,
            OpenAttributes::ByDriver { controller },
        ) {
            if status == ProtocolError::AlreadyStarted {
                return Ok(());
            } else {
                return Err(status);
            }
        };

        self.protocols
            .close_interface(controller, device_path_temp::Protocol::PROTOCOL_GUID, agent, Some(controller))
            .ok();

        Ok(())
    }

    /// Starts USB Bus support for the given controller.
    fn start(
        &self,
        agent: Handle,
        controller: Handle,
        remaining_device_path: Option<DevicePathWalker>,
    ) -> core::result::Result<(), ProtocolError> {
        log::trace!("USB Bus: driver_binding_start on controller {:?}", controller);

        let _parent_device_path = self.protocols.open_protocol::<device_path_temp::Protocol>(
            controller,
            agent,
            OpenAttributes::Shared,
        )?;

        let status = self.protocols.with_protocol::<status_code::StatusCodeProtocol, _>(|protocol| {
            protocol.report_status_code(
                EFI_PROGRESS_CODE,
                EFI_IO_BUS_USB | EFI_IOB_PC_INIT,
                0,
                patina::guid::CALLER_ID.as_efi_guid(),
            )
        })?;
        status.map_err(|status| ProtocolError::from(EfiError::from(status)))?;

        let bus_protocol_exists = self
            .protocols
            .with_protocol_on::<EfiUsbBusProtocol, _>(controller, |_| ())
            .is_ok();

        if bus_protocol_exists {
            //
            // USB bus driver needs to control the recursive connect policy of the bus, only those wanted
            // USB child devices will be recursively connected.
            // remaining_device_path indicates the child USB device which users want to fully recursively connect this time.
            // All wanted USB child devices will be remembered by the USB bus driver itself.
            // If remaining_device_path is NULL, all the USB child devices in the USB bus are wanted devices.
            //
            // Save the passed in RemainingDevicePath this time
            //
            let remaining_device_path = match remaining_device_path {
                None => None,
                Some(remaining_device_path) => {
                    let mut path = Vec::new();
                    for node in remaining_device_path {
                        let header = node.header();
                        if path.is_empty()
                            && header.r#type == device_path_temp::TYPE_END
                            && header.sub_type == device_path_temp::END_ENTIRE_DEVICE_PATH_SUBTYPE
                        {
                            return Ok(());
                        }
                        path.push(header.r#type);
                        path.push(header.sub_type);
                        path.extend_from_slice(&header.length);
                        path.extend_from_slice(node.data());
                    }
                    if path.is_empty() {
                        return Err(ProtocolError::InvalidParameter);
                    }
                    Some(path)
                }
            };

            let bus_id = self.protocols.interface_on_handle(controller, EfiUsbBusProtocol::PROTOCOL_GUID)?;
            // SAFETY: This private protocol was installed as the `bus_id` field of the bus owned
            // by this driver, and DriverBinding::start serializes updates to its policy.
            let bus_id = unsafe { &mut *bus_id.as_raw().cast::<EfiUsbBusProtocol>() };
            let remaining_device_path = remaining_device_path.as_ref().map(|path| {
                // SAFETY: The bytes were copied from a validated DevicePathWalker and remain alive
                // for the duration of the helper call. Device path headers have byte alignment.
                unsafe { &*path.as_ptr().cast::<device_path_temp::EfiDevicePathProtocol>() }
            });
            // SAFETY: The remaining path is complete and valid, and `bus_id` exclusively refers to
            // the private bus interface for this serialized start operation.
            unsafe { usb_bus_add_wanted_usb_io_dp(bus_id, remaining_device_path) }?;
            usb_bus_recursively_connect_wanted_usb_io(*self.protocols, *self.drivers, bus_id)?;
            return Ok(());
        }

        usb_bus_build_protocol(self.protocols, agent, controller, remaining_device_path)?;

        Ok(())
    }

    /// Stops USB Bus support for the given controller.
    fn stop(
        &self,
        _agent: Handle,
        controller: Handle,
        _children: &[Handle],
    ) -> core::result::Result<(), ProtocolError> {
        log::trace!("USB Bus: driver_binding_stop on controller {:?}", controller);

        Ok(())
    }
}

/// USB bus Patina component.
///
/// When dispatched, installs a UEFI Driver Binding that produces the UsbIo protocol.
#[derive(Default)]
pub struct UsbBusComponent;

#[component]
impl UsbBusComponent {
    /// Creates a new instance of the component.
    pub fn new() -> Self {
        Self
    }

    fn entry_point(self, protocols: Service<dyn ProtocolServices>, drivers: Service<dyn DriverServices>) -> Result<()> {
        let _agent = protocols.install_driver_binding(UsbBusDriver { protocols, drivers })?;
        Ok(())
    }
}

/// Installs the USB bus driver binding using the provided protocol services.
fn usb_bus_build_protocol(
    protocols: Service<dyn ProtocolServices>,
    agent: Handle,
    controller: Handle,
    remaining_device_path: Option<DevicePathWalker>,
) -> core::result::Result<(), ProtocolError> {
    let _ = remaining_device_path;

    let device_path = protocols.open_interface(
        controller,
        device_path_temp::Protocol::PROTOCOL_GUID,
        agent,
        OpenAttributes::ByDriver { controller },
    )?;

    let usb2_hc = match protocols.open_interface(
        controller,
        Usb2HcProtocol::PROTOCOL_GUID,
        agent,
        OpenAttributes::ByDriver { controller },
    ) {
        Ok(usb2_hc) => usb2_hc,
        Err(status) => {
            protocols
                .close_interface(controller, device_path_temp::Protocol::PROTOCOL_GUID, agent, Some(controller))
                .ok();
            return Err(status);
        }
    };

    let device_path_ptr = device_path.as_raw().cast::<efi::protocols::device_path::Protocol>();
    let usb2_hc_ptr = usb2_hc.as_raw().cast::<Usb2HcProtocol>();

    let mut bus = Box::new(UsbBus {
        signature: USB_BUS_SIGNATURE as usize,
        bus_id: EfiUsbBusProtocol { reserved: 0 },
        host_handle: controller.as_raw(),
        device_path: device_path_ptr,
        usb2_hc: usb2_hc_ptr,
        max_devices: USB_MAX_DEVICES as u32,
        devices: [ptr::null_mut(); 256],
        wanted_usb_io_dp_list: UsbDevicePathList::default(),
    });

    // SAFETY: `usb2_hc_ptr` came from the interface registered for `Usb2HcProtocol` and
    // remains open by this driver for the lifetime of the bus.
    if unsafe { (*usb2_hc_ptr).major_revision == 0x3 } {
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
    let mut root_if = Box::new(MaybeUninit::<UsbInterface>::zeroed());

    let bus_ptr = &mut *bus as *mut UsbBus;
    let root_hub_ptr = &mut *root_hub as *mut UsbDevice;
    let root_if_ptr = root_if.as_mut_ptr();
    root_hub.bus = bus_ptr;
    root_hub.interfaces[0] = root_if_ptr;
    unsafe {
        ptr::addr_of_mut!((*root_if_ptr).signature).write(USB_INTERFACE_SIGNATURE as usize);
        ptr::addr_of_mut!((*root_if_ptr).device).write(root_hub_ptr);
        ptr::addr_of_mut!((*root_if_ptr).device_path).write(device_path_ptr);
    }
    bus.devices[0] = root_hub_ptr;

    let bus_id = ProtocolPtr::from_raw(ptr::from_mut(&mut bus.bus_id).cast()).ok_or(ProtocolError::InvalidParameter)?;
    if let Err(status) = protocols.install_interface(Some(controller), EfiUsbBusProtocol::PROTOCOL_GUID, bus_id) {
        protocols
            .close_interface(controller, Usb2HcProtocol::PROTOCOL_GUID, agent, Some(controller))
            .ok();
        protocols
            .close_interface(controller, device_path_temp::Protocol::PROTOCOL_GUID, agent, Some(controller))
            .ok();
        return Err(status);
    }

    Box::leak(bus);
    Box::leak(root_hub);
    Box::leak(root_if);

    Ok(())
}