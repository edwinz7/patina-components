//! USB Bus Driver — produces the UsbIo protocol on USB Bus devices.
//!
//! This crate implements a UEFI Driver Binding that consumes the USB IO protocol
//! and produces the UsbIo protocol for each USB Bus device it manages.
//!
//! ## License
//!
//! Copyright (c) Microsoft Corporation.
//!
//! SPDX-License-Identifier: Apache-2.0
//!
extern crate alloc;

//#[cfg(test)]
//pub(crate) mod test_stubs;

use alloc::{boxed::Box, vec::Vec};
use core::cell::Cell;
use core::ptr;
use r_efi::efi;

use patina::{
    component::{
        service::{
            Service,
            uefi_services::{
                driver_model::{driver::DriverServices, driver_binding::DriverBinding},
                event::EventServices,
                protocol::{Handle, OpenAttributes, ProtocolError, ProtocolPtr, ProtocolServices, ProtocolServicesExt},
                timer_event::TimerEventServices,
                timing::TimingServices,
                tpl::{Tpl, TplServices, TplServicesExt},
            },
        },
    },
    error::EfiError,
    pi::{
        protocol::status_code,
        status_code::{EFI_IO_BUS_USB, EFI_IOB_PC_DETECT, EFI_IOB_PC_INIT, EFI_PROGRESS_CODE, EFI_P_PC_ENABLE},
    },
    uefi::device_path::walker::DevicePathWalker,
    protocol::ProtocolInterface,
};

use crate::device_path_temp;
use crate::usb_2_host_controller::Protocol as Usb2HcProtocol;
use crate::usb_bus_defs::{
    EfiUsbBusProtocol, USB_BUS_SIGNATURE, USB_INTERFACE_SIGNATURE, USB_MAX_DEVICES, UsbBus, UsbDevice,
    UsbDevicePathList, UsbHubServices, UsbInterface, usb_bus_from_this_mut, usb_interface_from_usb_io_mut,
};
use crate::usb_enumer::usb_remove_device;
use crate::usb_hub::{usb_root_hub_init, usb_root_hub_release};
use crate::usb_io_impl::new_usb_io_protocol;
use crate::usb_utility::{
    UsbIoProtocol, usb_bus_add_wanted_usb_io_dp, usb_bus_free_usb_dp_list,
    usb_bus_recursively_connect_wanted_usb_io,
};

pub struct UsbBusDriver {
    protocols: Service<dyn ProtocolServices>,
    drivers: Service<dyn DriverServices>,
    tpl: Service<dyn TplServices>,
    events: Service<dyn EventServices>,
    timers: Service<dyn TimerEventServices>,
    timing: Service<dyn TimingServices>,
}

impl UsbBusDriver {
    /// Creates a new USB bus driver bound to the given agent handle.
    pub fn new(
        protocols: Service<dyn ProtocolServices>,
        drivers: Service<dyn DriverServices>,
        tpl: Service<dyn TplServices>,
        events: Service<dyn EventServices>,
        timers: Service<dyn TimerEventServices>,
        timing: Service<dyn TimingServices>,
    ) -> Self {
        Self { protocols, drivers, tpl, events, timers, timing }
    }
}

/// Installs the USB bus driver binding using the provided protocol services.
fn usb_bus_build_protocol(
    protocols: Service<dyn ProtocolServices>,
    tpl: Service<dyn TplServices>,
    events: Service<dyn EventServices>,
    timers: Service<dyn TimerEventServices>,
    timing: Service<dyn TimingServices>,
    drivers: Service<dyn DriverServices>,
    agent: Handle,
    controller: Handle,
    remaining_device_path: Option<DevicePathWalker>,
) -> core::result::Result<(), ProtocolError> {
    let remaining_device_path = collect_device_path(remaining_device_path)?;

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
        hub_services: UsbHubServices {
            protocols: *protocols,
            agent,
            events: *events,
            timers: *timers,
            timing: *timing,
            tpl: *tpl,
            drivers: *drivers,
        },
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
        translator: Default::default(),
        parent_addr: 0,
        parent_if: ptr::null_mut(),
        parent_port: 0,
        tier: 0,
        connected: false.into(),
        disconnect_fail: false.into(),
    });
    let bus_ptr = &mut *bus as *mut UsbBus;
    let root_hub_ptr = &mut *root_hub as *mut UsbDevice;
    let mut root_if = Box::new(UsbInterface {
        signature: USB_INTERFACE_SIGNATURE as usize,
        device: root_hub_ptr,
        if_desc: ptr::null_mut(),
        if_setting: ptr::null_mut(),
        handle: ptr::null_mut(),
        usb_io: new_usb_io_protocol(),
        device_path: device_path_ptr,
        is_managed: Cell::new(false),
        is_hub: false,
        hub_api: None,
        num_of_port: 0,
        hub_notify: ptr::null_mut(),
        hub_ep: ptr::null_mut(),
        change_map: ptr::null_mut(),
        change_map_length: 0,
        max_speed: 0,
        poll_count: 0,
    });
    let root_if_ptr = &mut *root_if as *mut UsbInterface;
    root_hub.bus = bus_ptr;
    root_hub.interfaces[0] = root_if_ptr;

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

    let remaining_device_path = remaining_device_path.as_ref().map(|path| {
        // SAFETY: `collect_device_path` produced a complete path whose packed header has byte alignment.
        unsafe { &*path.as_ptr().cast::<device_path_temp::EfiDevicePathProtocol>() }
    });
    // SAFETY: `bus.bus_id` belongs to this exclusively owned bus and the path remains alive
    // throughout the call.
    if let Err(status) = unsafe { usb_bus_add_wanted_usb_io_dp(&mut bus.bus_id, remaining_device_path) } {
        protocols.uninstall_interface(controller, EfiUsbBusProtocol::PROTOCOL_GUID, bus_id).ok();
        protocols
            .close_interface(controller, Usb2HcProtocol::PROTOCOL_GUID, agent, Some(controller))
            .ok();
        protocols
            .close_interface(controller, device_path_temp::Protocol::PROTOCOL_GUID, agent, Some(controller))
            .ok();
        return Err(status);
    }

    let status = protocols
        .with_protocol::<status_code::StatusCodeProtocol, _>(|protocol| {
            protocol.report_status_code(
                EFI_PROGRESS_CODE,
                EFI_IO_BUS_USB | EFI_IOB_PC_DETECT,
                0,
                patina::guid::CALLER_ID.as_efi_guid(),
            )
        })
        .and_then(|status| status.map_err(|status| ProtocolError::from(EfiError::from(status))));
    if let Err(status) = status {
        protocols.uninstall_interface(controller, EfiUsbBusProtocol::PROTOCOL_GUID, bus_id).ok();
        protocols
            .close_interface(controller, Usb2HcProtocol::PROTOCOL_GUID, agent, Some(controller))
            .ok();
        protocols
            .close_interface(controller, device_path_temp::Protocol::PROTOCOL_GUID, agent, Some(controller))
            .ok();
        return Err(status);
    }

    // SAFETY: The root interface and bus allocations remain live until DriverBinding::stop.
    let status = unsafe { usb_root_hub_init(&mut root_if, &bus.hub_services) };
    if status != efi::Status::SUCCESS {
        protocols.uninstall_interface(controller, EfiUsbBusProtocol::PROTOCOL_GUID, bus_id).ok();
        protocols
            .close_interface(controller, Usb2HcProtocol::PROTOCOL_GUID, agent, Some(controller))
            .ok();
        protocols
            .close_interface(controller, device_path_temp::Protocol::PROTOCOL_GUID, agent, Some(controller))
            .ok();
        return Err(ProtocolError::from(EfiError::from(status)));
    }

    bus.devices[0] = root_hub_ptr;

    Box::leak(bus);
    Box::leak(root_hub);
    Box::leak(root_if);

    Ok(())
}

fn collect_device_path(
    remaining_device_path: Option<DevicePathWalker>,
) -> core::result::Result<Option<Vec<u8>>, ProtocolError> {
    let Some(remaining_device_path) = remaining_device_path else {
        return Ok(None);
    };

    let mut path = Vec::new();
    for node in remaining_device_path {
        let header = node.header();
        path.push(header.r#type);
        path.push(header.sub_type);
        path.extend_from_slice(&header.length);
        path.extend_from_slice(node.data());
    }
    if path.is_empty() {
        return Err(ProtocolError::InvalidParameter);
    }

    Ok(Some(path))
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

        let status = self.protocols.with_protocol::<status_code::StatusCodeProtocol, _>(|protocol| {
            protocol.report_status_code(
                EFI_PROGRESS_CODE,
                EFI_IO_BUS_USB | EFI_P_PC_ENABLE,
                0,
                patina::guid::CALLER_ID.as_efi_guid(),
            )
        })?;
        status.map_err(|status| ProtocolError::from(EfiError::from(status)))?;

        usb_bus_build_protocol(
            self.protocols,
            self.tpl,
            self.events,
            self.timers,
            self.timing,
            self.drivers,
            agent,
            controller,
            remaining_device_path,
        )?;

        Ok(())
    }

    /// Stops USB Bus support for the given controller.
    fn stop(
        &self,
        agent: Handle,
        controller: Handle,
        children: &[Handle],
    ) -> core::result::Result<(), ProtocolError> {
        log::trace!("USB Bus: driver_binding_stop on controller {:?}", controller);

        if !children.is_empty() {
            let mut result = Ok(());
            let _guard = self.tpl.raise(Tpl::Callback);
            for child in children {
                let Ok(usb_io) = self.protocols.interface_on_handle(*child, UsbIoProtocol::PROTOCOL_GUID) else {
                    continue;
                };
                // SAFETY: The USB I/O protocol was installed as the `usb_io` field of a live
                // interface. Stop runs at callback TPL, providing exclusive teardown access.
                let device = {
                    let Some(interface) = (unsafe {
                        usb_interface_from_usb_io_mut(&mut *usb_io.as_raw().cast::<efi::protocols::usb_io::Protocol>())
                    }) else {
                        result = Err(ProtocolError::InvalidParameter);
                        continue;
                    };
                    interface.device
                };
                if device.is_null() {
                    result = Err(ProtocolError::InvalidParameter);
                    continue;
                }
                let status = unsafe { usb_remove_device(device) };
                result = if status == efi::Status::SUCCESS {
                    Ok(())
                } else {
                    Err(ProtocolError::from(EfiError::from(status)))
                };
            }
            return result;
        }

        let bus_id = self.protocols.interface_on_handle(controller, EfiUsbBusProtocol::PROTOCOL_GUID)?;
        // SAFETY: The private protocol is embedded in the bus allocated by this driver, and stop
        // holds exclusive teardown access while mutating and eventually reclaiming it.
        let bus_id = unsafe { &mut *bus_id.as_raw().cast::<EfiUsbBusProtocol>() };
        let Some(bus) = (unsafe { usb_bus_from_this_mut(bus_id) }) else {
            return Err(ProtocolError::InvalidParameter);
        };
        let bus = bus as *mut UsbBus;
        let root_hub = unsafe { (*bus).devices[0] };
        let root_interface = unsafe { root_hub.as_ref() }
            .and_then(|root_hub| unsafe { root_hub.interfaces[0].as_mut() })
            .ok_or(ProtocolError::InvalidParameter)?;

        {
            let _guard = self.tpl.raise(Tpl::Callback);
            let max_devices = unsafe { ((*bus).max_devices as usize).min((*bus).devices.len()) };
            let mut removal_error = None;
            for index in 1..max_devices {
                let device = unsafe { (*bus).devices[index] };
                if device.is_null() {
                    continue;
                }
                let status = unsafe { usb_remove_device(device) };
                if status != efi::Status::SUCCESS {
                    removal_error = Some(ProtocolError::from(EfiError::from(status)));
                }
            }
            if let Some(error) = removal_error {
                return Err(error);
            }
        }

        let status = usb_root_hub_release(root_interface, unsafe { &(*bus).hub_services });
        if status != efi::Status::SUCCESS {
            return Err(ProtocolError::from(EfiError::from(status)));
        }
        usb_bus_free_usb_dp_list(Some(unsafe { &mut (*bus).wanted_usb_io_dp_list }))?;

        let bus_protocol = ProtocolPtr::from_raw(unsafe { ptr::from_mut(&mut (*bus).bus_id).cast() })
            .ok_or(ProtocolError::InvalidParameter)?;
        self.protocols
            .uninstall_interface(controller, EfiUsbBusProtocol::PROTOCOL_GUID, bus_protocol)?;
        self.protocols
            .close_interface(controller, Usb2HcProtocol::PROTOCOL_GUID, agent, Some(controller))?;
        self.protocols
            .close_interface(controller, device_path_temp::Protocol::PROTOCOL_GUID, agent, Some(controller))
            .ok();

        let root_interface = root_interface as *mut UsbInterface;
        drop(unsafe { Box::from_raw(root_interface) });
        drop(unsafe { Box::from_raw(root_hub) });
        drop(unsafe { Box::from_raw(bus) });

        Ok(())
    }
}