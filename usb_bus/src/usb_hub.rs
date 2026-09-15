//! USB hub definitions translated from `UsbHub.h`.

// TODO: replace efi::Status codes with appropriate patina
// versions.

#![allow(dead_code)]

extern crate alloc;
use alloc::{vec, vec::Vec, boxed::Box};
use core::{ffi::c_void, mem, ptr, time::Duration};
use r_efi::{efi, efi::protocols::usb_io};
use patina::{
    component::service::uefi_services::{
        event::{Event, EventServices, EventServicesExt},
        timer_event::{TimerEventServices, TimerEventServicesExt, TimerType},
        timing::TimingServices,
        tpl::Tpl as ServiceTpl,
    },
    error::EfiError,
};

use crate::usb_2_host_controller::{
    UsbDataDirection, UsbPortFeature, UsbPortStatus,
    USB_PORT_STAT_C_CONNECTION, USB_PORT_STAT_C_ENABLE,
    USB_PORT_STAT_C_OVERCURRENT, USB_PORT_STAT_C_RESET,
    USB_PORT_STAT_C_SUSPEND, USB_PORT_STAT_ENABLE, USB_PORT_STAT_RESET,
    EFI_USB_SPEED_HIGH, EFI_USB_SPEED_SUPER,
};
use crate::usb_bus_defs::{
    UsbDevice, UsbInterface,
    USB_ENUM_POLL_MINIMUM_ATTEMPTS, USB_SET_PORT_RECOVERY_STALL,
    USB_SET_PORT_RESET_STALL, USB_SET_ROOT_PORT_ENABLE_STALL,
    USB_SET_ROOT_PORT_RESET_STALL, USB_CLR_ROOT_PORT_RESET_STALL,
    USB_SET_PORT_POWER_STALL, USB_WAIT_PORT_STS_CHANGE_STALL,
    USB_HUB_POLL_INTERVAL, USB_ROOTHUB_POLL_INTERVAL,
};
use crate::usb_desc::{usb_ctrl_request, usb_io_clear_feature};
use crate::usb_utility::{
    usb_hc_clear_root_hub_port_feature, usb_hc_get_capability,
    usb_hc_get_root_hub_port_status, usb_hc_set_root_hub_port_feature,
};
use crate::usb_enumer::{usb_hub_enumeration, usb_root_hub_enumeration};

pub const USB_ENDPOINT_ADDR_MASK: u8 = 0x7f;
pub const USB_ENDPOINT_TYPE_MASK: u8 = 0x03;

pub const USB_DESC_TYPE_HUB: u8 = 0x29;
pub const USB_DESC_TYPE_HUB_SUPER_SPEED: u8 = 0x2a;

pub const USB_HUB_TARGET_HUB: usize = 0;
pub const USB_HUB_TARGET_PORT: usize = 3;

pub const USB_HUB_REQ_GET_STATUS: u8 = 0;
pub const USB_HUB_REQ_CLEAR_FEATURE: u8 = 1;
pub const USB_HUB_REQ_SET_FEATURE: u8 = 3;
pub const USB_HUB_REQ_GET_DESC: u8 = 6;
pub const USB_HUB_REQ_SET_DESC: u8 = 7;
pub const USB_HUB_REQ_CLEAR_TT: u8 = 8;
pub const USB_HUB_REQ_RESET_TT: u8 = 9;
pub const USB_HUB_REQ_GET_TT_STATE: u8 = 10;
pub const USB_HUB_REQ_STOP_TT: u8 = 11;
pub const USB_HUB_REQ_SET_DEPTH: u8 = 12;

pub const USB_HUB_C_HUB_LOCAL_POWER: u16 = 0;
pub const USB_HUB_C_HUB_OVER_CURRENT: u16 = 1;
pub const USB_HUB_PORT_CONNECTION: u16 = 0;
pub const USB_HUB_PORT_ENABLE: u16 = 1;
pub const USB_HUB_PORT_SUSPEND: u16 = 2;
pub const USB_HUB_PORT_OVER_CURRENT: u16 = 3;
pub const USB_HUB_PORT_RESET: u16 = 4;
pub const USB_HUB_PORT_LINK_STATE: u16 = 5;
pub const USB_HUB_PORT_POWER: u16 = 8;
pub const USB_HUB_PORT_LOW_SPEED: u16 = 9;
pub const USB_HUB_C_PORT_CONNECT: u16 = 16;
pub const USB_HUB_C_PORT_ENABLE: u16 = 17;
pub const USB_HUB_C_PORT_SUSPEND: u16 = 18;
pub const USB_HUB_C_PORT_OVER_CURRENT: u16 = 19;
pub const USB_HUB_C_PORT_RESET: u16 = 20;
pub const USB_HUB_PORT_TEST: u16 = 21;
pub const USB_HUB_PORT_INDICATOR: u16 = 22;
pub const USB_HUB_C_PORT_LINK_STATE: u16 = 25;
pub const USB_HUB_PORT_REMOTE_WAKE_MASK: u16 = 27;
pub const USB_HUB_BH_PORT_RESET: u16 = 28;
pub const USB_HUB_C_BH_PORT_RESET: u16 = 29;

pub const USB_SS_PORT_STAT_C_BH_RESET: u16 = 0x0020;
pub const USB_SS_PORT_STAT_C_PORT_LINK_STATE: u16 = 0x0040;

pub const USB_HUB_GANG_POWER_CTRL: u8 = 0;
pub const USB_HUB_PORT_POWER_CTRL: u8 = 1;
pub const USB_HUB_STAT_LOCAL_POWER: u8 = 0x01;
pub const USB_HUB_STAT_OVER_CURRENT: u8 = 0x02;
pub const USB_HUB_STAT_C_LOCAL_POWER: u8 = 0x01;
pub const USB_HUB_STAT_C_OVER_CURRENT: u8 = 0x02;
pub const USB_HUB_CLASS_CODE: u8 = 0x09;
pub const USB_HUB_SUBCLASS_CODE: u8 = 0x00;
pub const USB_WAIT_PORT_STS_CHANGE_LOOP: usize = 5000;
pub const USB_REQ_TYPE_CLASS: usize = 0x20;

/// Returns the endpoint number without its direction bit.
pub const fn usb_endpoint_addr(endpoint_address: u8) -> u8 {
    endpoint_address & USB_ENDPOINT_ADDR_MASK
}

/// Returns the transfer type from a USB endpoint attributes byte.
pub const fn usb_endpoint_type(attributes: u8) -> u8 {
    attributes & USB_ENDPOINT_TYPE_MASK
}

/// USB hub descriptor header and fixed fields.
#[repr(C, packed)]
#[derive(Clone, Copy, Debug, Default)]
pub struct UsbHubDescriptor {
    pub length: u8,
    pub descriptor_type: u8,
    pub num_ports: u8,
    pub hub_character: u16,
    pub power_on_to_power_good: u8,
    pub hub_controller_current: u8,
    pub filler: [u8; 16],
}

/// A hub status-change bit and the feature that acknowledges it.
#[repr(C)]
pub struct UsbChangeFeatureMap {
    pub changed_bit: u16,
    pub feature: UsbPortFeature,
}

struct UsbHubInterruptContext {
    pub interface: *mut UsbInterface,
    pub events: &'static dyn EventServices,
}

/// Sets the hub depth for a SuperSpeed hub.
pub unsafe fn usb_hub_ctrl_set_hub_depth(hub: &mut UsbDevice, depth: u16) -> efi::Status {
    unsafe { usb_ctrl_request(hub, UsbDataDirection::NoData, USB_REQ_TYPE_CLASS, USB_HUB_TARGET_HUB, USB_HUB_REQ_SET_DEPTH as usize, depth, 0, core::ptr::null_mut(), 0) }
}

/// Clears a hub feature.
pub unsafe fn usb_hub_ctrl_clear_hub_feature(hub: &mut UsbDevice, feature: u16) -> efi::Status {
    unsafe { usb_ctrl_request(hub, UsbDataDirection::NoData, USB_REQ_TYPE_CLASS, USB_HUB_TARGET_HUB, USB_HUB_REQ_CLEAR_FEATURE as usize, feature, 0, core::ptr::null_mut(), 0) }
}

/// Clears a feature on a hub port. The USB hub port number is one-based.
pub unsafe fn usb_hub_ctrl_clear_port_feature(hub: &mut UsbDevice, port: u8, feature: u16) -> efi::Status {
    unsafe { usb_ctrl_request(hub, UsbDataDirection::NoData, USB_REQ_TYPE_CLASS, USB_HUB_TARGET_PORT, USB_HUB_REQ_CLEAR_FEATURE as usize, feature, port as u16 + 1, core::ptr::null_mut(), 0) }
}

/// Clears a transaction-translator buffer for a failed split transaction.
pub unsafe fn usb_hub_ctrl_clear_tt_buffer(
    hub: &mut UsbDevice,
    port: u8,
    device_address: u16,
    endpoint_number: u16,
    endpoint_type: u16,
) -> efi::Status {
    let value = (endpoint_number & 0x0f)
        | (device_address << 4)
        | ((endpoint_type & 0x03) << 11)
        | ((endpoint_number & 0x80) << 15);
    unsafe { usb_ctrl_request(hub, UsbDataDirection::NoData, USB_REQ_TYPE_CLASS, USB_HUB_TARGET_PORT, USB_HUB_REQ_CLEAR_TT as usize, value, port as u16 + 1, core::ptr::null_mut(), 0) }
}

/// Retrieves a hub descriptor into the supplied buffer.
pub unsafe fn usb_hub_ctrl_get_hub_desc(hub: &mut UsbDevice, buffer: *mut c_void, length: usize) -> efi::Status {
    let descriptor_type = if hub.speed == EFI_USB_SPEED_SUPER {
        USB_DESC_TYPE_HUB_SUPER_SPEED
    } else {
        USB_DESC_TYPE_HUB
    };
    unsafe { usb_ctrl_request(hub, UsbDataDirection::In, USB_REQ_TYPE_CLASS, USB_HUB_TARGET_HUB, USB_HUB_REQ_GET_DESC as usize, (descriptor_type as u16) << 8, 0, buffer, length) }
}

/// Retrieves the hub status bitmap.
pub unsafe fn usb_hub_ctrl_get_hub_status(hub: &mut UsbDevice, state: &mut u32) -> efi::Status {
    unsafe { usb_ctrl_request(hub, UsbDataDirection::In, USB_REQ_TYPE_CLASS, USB_HUB_TARGET_HUB, USB_HUB_REQ_GET_STATUS as usize, 0, 0, (state as *mut u32).cast(), mem::size_of::<u32>()) }
}

/// Retrieves a hub port status. The USB hub port number is one-based.
pub unsafe fn usb_hub_ctrl_get_port_status(hub: &mut UsbDevice, port: u8, state: &mut UsbPortStatus) -> efi::Status {
    unsafe { usb_ctrl_request(hub, UsbDataDirection::In, USB_REQ_TYPE_CLASS, USB_HUB_TARGET_PORT, USB_HUB_REQ_GET_STATUS as usize, 0, port as u16 + 1, (state as *mut UsbPortStatus).cast(), mem::size_of::<UsbPortStatus>()) }
}

/// Sets a feature on a hub port. The USB hub port number is one-based.
pub unsafe fn usb_hub_ctrl_set_port_feature(hub: &mut UsbDevice, port: u8, feature: u16) -> efi::Status {
    unsafe { usb_ctrl_request(hub, UsbDataDirection::NoData, USB_REQ_TYPE_CLASS, USB_HUB_TARGET_PORT, USB_HUB_REQ_SET_FEATURE as usize, feature as u16, port as u16 + 1, core::ptr::null_mut(), 0) }
}

/// Reads the complete hub descriptor after obtaining its variable length.
pub unsafe fn usb_hub_read_desc(hub: &mut UsbDevice) -> Option<Vec<u8>> {
    let mut header = [0u8; 2];
    let status = unsafe { usb_hub_ctrl_get_hub_desc(hub, header.as_mut_ptr().cast(), header.len()) };
    if status != efi::Status::SUCCESS || header[0] < 2 {
        return None;
    }
    let mut descriptor = vec![0u8; header[0] as usize];
    let status = unsafe { usb_hub_ctrl_get_hub_desc(hub, descriptor.as_mut_ptr().cast(), descriptor.len()) };
    (status == efi::Status::SUCCESS).then_some(descriptor)
}

/// Acknowledges hub-level status changes.
pub unsafe fn usb_hub_ack_hub_status(hub: &mut UsbDevice) -> efi::Status {
    let mut state = 0;
    let status = unsafe { usb_hub_ctrl_get_hub_status(hub, &mut state) };
    if status != efi::Status::SUCCESS {
        return status;
    }
    if state & (USB_HUB_STAT_C_LOCAL_POWER as u32) != 0 {
        let _ = unsafe { usb_hub_ctrl_clear_hub_feature(hub, USB_HUB_C_HUB_LOCAL_POWER) };
    }
    if state & (USB_HUB_STAT_C_OVER_CURRENT as u32) != 0 {
        let _ = unsafe { usb_hub_ctrl_clear_hub_feature(hub, USB_HUB_C_HUB_OVER_CURRENT) };
    }
    efi::Status::SUCCESS
}

/// Returns whether an interface describes a USB hub function.
pub unsafe fn usb_is_hub_interface(interface: &UsbInterface) -> bool {
    let Some(setting) = (unsafe { interface.if_setting.as_ref() }) else {
        return false;
    };
    setting.descriptor.interface_class == USB_HUB_CLASS_CODE
        && setting.descriptor.interface_sub_class == USB_HUB_SUBCLASS_CODE
}

/// Processes a hub interrupt completion.
pub unsafe extern "efiapi" fn usb_on_hub_interrupt(
    data: *mut c_void,
    data_length: usize,
    context: *mut c_void,
    result: u32,
) -> efi::Status {
    if context.is_null() {
        return efi::Status::INVALID_PARAMETER;
    }

    let context = unsafe { &mut *(context as *mut UsbHubInterruptContext) };
    let Some(interface) = (unsafe { context.interface.as_mut() }) else {
        return efi::Status::INVALID_PARAMETER;
    };
    let Some(endpoint) = (unsafe { interface.hub_ep.as_ref() }) else {
        return efi::Status::DEVICE_ERROR;
    };
    let endpoint_address = endpoint.descriptor.endpoint_address;

    if result != usb_io::NOERROR {
        if result & usb_io::ERR_STALL != 0 {
            let _ = unsafe {
                usb_io_clear_feature(
                    &mut interface.usb_io,
                    2,
                    0,
                    endpoint_address as u16,
                )
            };
        }

        let status = unsafe {
            (interface.usb_io.async_interrupt_transfer)(
                &mut interface.usb_io,
                endpoint_address,
                false.into(),
                0,
                0,
                None,
                core::ptr::null_mut(),
            )
        };
        if status != efi::Status::SUCCESS {
            return status;
        }

        return unsafe {
            (interface.usb_io.async_interrupt_transfer)(
                &mut interface.usb_io,
                endpoint_address,
                true.into(),
                USB_HUB_POLL_INTERVAL as usize,
                interface.num_of_port as usize / 8 + 1,
                Some(usb_on_hub_interrupt),
                context as *mut UsbHubInterruptContext as *mut c_void,
            )
        };
    }

    if data.is_null() || data_length == 0 {
        return efi::Status::SUCCESS;
    }

    let change_map = unsafe { core::slice::from_raw_parts(data.cast::<u8>(), data_length) };
    let mut copied_map = Vec::new();
    if copied_map.try_reserve_exact(data_length).is_err() {
        return efi::Status::OUT_OF_RESOURCES;
    }
    copied_map.extend_from_slice(change_map);
    let change_map = copied_map.into_boxed_slice();
    if !interface.change_map.is_null() && interface.change_map_length != 0 {
        drop(unsafe {
            Box::from_raw(core::ptr::slice_from_raw_parts_mut(
                interface.change_map,
                interface.change_map_length,
            ))
        });
    }
    interface.change_map = Box::into_raw(change_map).cast::<u8>();
    interface.change_map_length = data_length;
    let Some(event) = Event::from_raw(interface.hub_notify) else {
        return efi::Status::INVALID_PARAMETER;
    };
    if context.events.signal_event(event).is_err() {
        return efi::Status::INVALID_PARAMETER;
    }

    efi::Status::SUCCESS
}

/// Initializes a normal hub and registers its Boot Services event and interrupt poll.
pub unsafe fn usb_hub_init(
    interface: &mut UsbInterface,
    events: &'static dyn EventServices,
    timing: &dyn TimingServices,
) -> efi::Status {
    if !unsafe { usb_is_hub_interface(interface) } {
        return efi::Status::DEVICE_ERROR;
    }
    let Some(setting) = (unsafe { interface.if_setting.as_ref() }) else {
        return efi::Status::INVALID_PARAMETER;
    };
    let endpoint = (0..setting.descriptor.num_endpoints)
        .filter_map(|index| unsafe { setting.endpoints.add(index as usize).as_ref() })
        .find(|endpoint| unsafe {
            (***endpoint).descriptor.endpoint_address & 0x80 != 0
                && usb_endpoint_type((***endpoint).descriptor.attributes) == 3
        })
        .copied();
    let Some(endpoint) = endpoint else { return efi::Status::DEVICE_ERROR };
    let Some(device) = (unsafe { interface.device.as_mut() }) else {
        return efi::Status::INVALID_PARAMETER;
    };
    let descriptor = unsafe { usb_hub_read_desc(device) };
    let Some(descriptor) = descriptor else { return efi::Status::DEVICE_ERROR };
    if descriptor.len() < 3 {
        return efi::Status::DEVICE_ERROR;
    }

    if device.speed == EFI_USB_SPEED_SUPER {
        let depth = device.tier.saturating_sub(1) as u16;
        let _ = unsafe { usb_hub_ctrl_set_hub_depth(device, depth) };
        for port in 0..descriptor[2] {
            let _ = unsafe {
                usb_hub_ctrl_set_port_feature(
                    device,
                    port,
                    USB_HUB_PORT_REMOTE_WAKE_MASK,
                )
            };
        }
    } else {
        for port in 0..descriptor[2] {
            let _ = unsafe {
                usb_hub_ctrl_set_port_feature(device, port, USB_HUB_PORT_POWER)
            };
        }

        if descriptor.len() > 5 && descriptor[5] != 0 {
            let _ = timing.stall(Duration::from_micros(
                descriptor[5] as u64 * USB_SET_PORT_POWER_STALL,
            ));
        }
        let _ = unsafe { usb_hub_ack_hub_status(device) };
    }

    interface.is_hub = true.into();
    interface.num_of_port = descriptor[2];
    interface.hub_ep = endpoint as *const _ as *mut _;
    interface.hub_interrupt_context = core::ptr::null_mut();

    let interface_ptr = interface as *mut UsbInterface as usize;
    let event = match events.on_event(ServiceTpl::Callback, move || unsafe {
        usb_hub_enumeration(ptr::null_mut(), interface_ptr as *mut c_void)
    }) {
        Ok(event) => event,
        Err(_) => return efi::Status::INVALID_PARAMETER,
    };
    interface.hub_notify = event.as_raw();

    let Some(endpoint) = (unsafe { interface.hub_ep.as_ref() }) else {
        let _ = events.close_event(event);
        interface.hub_notify = core::ptr::null_mut();
        return efi::Status::DEVICE_ERROR;
    };
    let interrupt_context = Box::new(UsbHubInterruptContext {
        interface: interface as *mut UsbInterface,
        events,
    });
    let interrupt_context = Box::into_raw(interrupt_context);
    interface.hub_interrupt_context = interrupt_context.cast();
    let status = unsafe {
        (interface.usb_io.async_interrupt_transfer)(
            &mut interface.usb_io,
            endpoint.descriptor.endpoint_address,
            true.into(),
            USB_HUB_POLL_INTERVAL as usize,
            interface.num_of_port as usize / 8 + 1,
            Some(usb_on_hub_interrupt),
            interrupt_context.cast(),
        )
    };
    if status != efi::Status::SUCCESS {
        drop(unsafe { Box::from_raw(interrupt_context) });
        interface.hub_interrupt_context = core::ptr::null_mut();
        let _ = events.close_event(event);
        interface.hub_notify = core::ptr::null_mut();
        interface.is_hub = false.into();
    }
    status
}
/// Retrieves and acknowledges a normal hub port status.
pub unsafe fn usb_hub_get_port_status(
    interface: &mut UsbInterface,
    port: u8,
    status: &mut UsbPortStatus,
) -> efi::Status {
    let Some(device) = (unsafe { interface.device.as_mut() }) else {
        return efi::Status::INVALID_PARAMETER;
    };
    unsafe { usb_hub_ctrl_get_port_status(device, port, status) }
}

/// Clears all reported change bits on a normal hub port.
pub unsafe fn usb_hub_clear_port_change(interface: &mut UsbInterface, port: u8) {
    let mut status = UsbPortStatus { port_status: 0, port_change_status: 0 };
    if unsafe { usb_hub_get_port_status(interface, port, &mut status) } != efi::Status::SUCCESS {
        return;
    }
    for (changed_bit, feature) in [
        (USB_PORT_STAT_C_CONNECTION, UsbPortFeature::ConnectChange),
        (USB_PORT_STAT_C_ENABLE, UsbPortFeature::EnableChange),
        (USB_PORT_STAT_C_SUSPEND, UsbPortFeature::SuspendChange),
        (USB_PORT_STAT_C_OVERCURRENT, UsbPortFeature::OverCurrentChange),
        (USB_PORT_STAT_C_RESET, UsbPortFeature::ResetChange),
    ] {
        if status.port_change_status & changed_bit != 0 {
            if let Some(device) = unsafe { interface.device.as_mut() } {
                let _ = unsafe { usb_hub_ctrl_clear_port_feature(device, port, feature as u16) };
            }
        }
    }
}

/// Sets a feature on a normal hub port.
pub unsafe fn usb_hub_set_port_feature(interface: &mut UsbInterface, port: u8, feature: UsbPortFeature) -> efi::Status {
    let Some(device) = (unsafe { interface.device.as_mut() }) else {
        return efi::Status::INVALID_PARAMETER;
    };
    unsafe { usb_hub_ctrl_set_port_feature(device, port, feature as u16) }
}

/// Clears a feature on a normal hub port.
pub unsafe fn usb_hub_clear_port_feature(interface: &mut UsbInterface, port: u8, feature: UsbPortFeature) -> efi::Status {
    let Some(device) = (unsafe { interface.device.as_mut() }) else {
        return efi::Status::INVALID_PARAMETER;
    };
    unsafe { usb_hub_ctrl_clear_port_feature(device, port, feature as u16) }
}

/// Resets a normal hub port and waits for the reset-change indication.
unsafe fn usb_hub_reset_port(
    interface: &mut UsbInterface,
    port: u8,
    timing: &dyn TimingServices,
) -> efi::Status {
    let status = unsafe { usb_hub_set_port_feature(interface, port, UsbPortFeature::Reset) };
    if status != efi::Status::SUCCESS {
        return status;
    }

    // Per USB 2.0, the reset signal must be driven for the full reset pulse
    // duration before checking for the reset-complete change bit.
    let _ = timing.stall(Duration::from_micros(USB_SET_PORT_RESET_STALL));

    for _ in 0..USB_WAIT_PORT_STS_CHANGE_LOOP {
        let mut port_status = UsbPortStatus { port_status: 0, port_change_status: 0 };
        let status = unsafe { usb_hub_get_port_status(interface, port, &mut port_status) };
        if status != efi::Status::SUCCESS {
            return status;
        }
        if port_status.port_change_status & USB_PORT_STAT_C_RESET != 0 {
            let _ = timing.stall(Duration::from_micros(USB_SET_PORT_RECOVERY_STALL));
            return efi::Status::SUCCESS;
        }
        let _ = timing.stall(Duration::from_micros(USB_WAIT_PORT_STS_CHANGE_STALL));
    }
    efi::Status::TIMEOUT
}

/// Releases normal-hub state and closes its Boot Services event.
pub fn usb_hub_release(interface: &mut UsbInterface, events: &dyn EventServices) -> efi::Status {
    if !interface.hub_ep.is_null() {
        let status = unsafe {
            (interface.usb_io.async_interrupt_transfer)(
                &mut interface.usb_io,
                (*interface.hub_ep).descriptor.endpoint_address,
                false.into(),
                USB_HUB_POLL_INTERVAL as usize,
                0,
                None,
                core::ptr::null_mut(),
            )
        };
        if status != efi::Status::SUCCESS {
            return status;
        }
    }

    if !interface.hub_notify.is_null() {
        let Some(event) = Event::from_raw(interface.hub_notify) else {
            return efi::Status::INVALID_PARAMETER;
        };
        if events.close_event(event).is_err() {
            return efi::Status::INVALID_PARAMETER;
        }
    }

    if !interface.hub_interrupt_context.is_null() {
        drop(unsafe {
            Box::from_raw(interface.hub_interrupt_context as *mut UsbHubInterruptContext)
        });
    }

    interface.is_hub = false.into();
    interface.hub_api = core::ptr::null_mut();
    interface.hub_ep = core::ptr::null_mut();
    interface.hub_interrupt_context = core::ptr::null_mut();
    interface.hub_notify = core::ptr::null_mut();
    efi::Status::SUCCESS
}

/// Initializes a root hub and configures its periodic timer.
pub unsafe fn usb_root_hub_init(
    interface: &mut UsbInterface,
    events: &'static dyn EventServices,
    timers: &'static dyn TimerEventServices,
) -> core::result::Result<(), EfiError> {
    let Some(device) = (unsafe { interface.device.as_ref() }) else {
        return Err(EfiError::InvalidParameter);
    };
    let Some(bus) = (unsafe { device.bus.as_mut() }) else {
        return Err(EfiError::InvalidParameter);
    };
    let mut max_speed = 0;
    let mut num_ports = 0;
    let mut support_64 = 0;
    let status = unsafe { usb_hc_get_capability(bus, &mut max_speed, &mut num_ports, &mut support_64) };
    if status != efi::Status::SUCCESS {
        return Err(EfiError::from(status));
    }

    // The original MU_BASECORE implementation initializes the root-hub interface
    // state before the timer is started and then waits for a minimum number of
    // poll cycles to avoid delaying enumeration by one timer interval.
    interface.is_hub = true.into();
    interface.hub_api = core::ptr::null_mut();
    interface.hub_ep = core::ptr::null_mut();
    interface.max_speed = max_speed;
    interface.num_of_port = num_ports;
    interface.poll_count = 0;

    let interface_ptr = interface as *mut UsbInterface as usize;
    let event = match timers.on_timer_event(ServiceTpl::Callback, move || unsafe {
        usb_root_hub_enumeration(ptr::null_mut(), interface_ptr as *mut c_void)
    }) {
        Ok(event) => event,
        Err(error) => return Err(EfiError::from(error)),
    };
    interface.hub_notify = event.as_raw();

    if let Err(error) = events.signal_event(event) {
        let _ = events.close_event(event);
        interface.hub_notify = core::ptr::null_mut();
        interface.is_hub = false.into();
        return Err(EfiError::from(error));
    }

    if let Err(error) = timers.set_timer(
        event,
        TimerType::Periodic(Duration::from_nanos(USB_ROOTHUB_POLL_INTERVAL * 100)),
    ) {
        let _ = events.close_event(event);
        interface.hub_notify = core::ptr::null_mut();
        interface.is_hub = false.into();
        return Err(EfiError::from(error));
    }

    while interface.poll_count < USB_ENUM_POLL_MINIMUM_ATTEMPTS {
        core::hint::spin_loop();
    }

    Ok(())
}

/// Reads a root-hub port status through the host-controller protocol.
pub unsafe fn usb_root_hub_get_port_status(interface: &mut UsbInterface, port: u8, status: &mut UsbPortStatus) -> efi::Status {
    let Some(device) = (unsafe { interface.device.as_ref() }) else { return efi::Status::INVALID_PARAMETER };
    let Some(bus) = (unsafe { device.bus.as_mut() }) else { return efi::Status::INVALID_PARAMETER };
    unsafe { usb_hc_get_root_hub_port_status(bus, port, status) }
}

/// Clears all reported change bits on a root-hub port.
pub unsafe fn usb_root_hub_clear_port_change(interface: &mut UsbInterface, port: u8) {
    let mut status = UsbPortStatus { port_status: 0, port_change_status: 0 };
    if unsafe { usb_root_hub_get_port_status(interface, port, &mut status) } != efi::Status::SUCCESS {
        return;
    }
    for (changed_bit, feature) in [
        (USB_PORT_STAT_C_CONNECTION, UsbPortFeature::ConnectChange),
        (USB_PORT_STAT_C_ENABLE, UsbPortFeature::EnableChange),
        (USB_PORT_STAT_C_SUSPEND, UsbPortFeature::SuspendChange),
        (USB_PORT_STAT_C_OVERCURRENT, UsbPortFeature::OverCurrentChange),
        (USB_PORT_STAT_C_RESET, UsbPortFeature::ResetChange),
    ] {
        if status.port_change_status & changed_bit != 0 {
            let _ = unsafe { usb_root_hub_clear_port_feature(interface, port, feature) };
        }
    }
}

/// Sets a root-hub port feature.
pub unsafe fn usb_root_hub_set_port_feature(interface: &mut UsbInterface, port: u8, feature: UsbPortFeature) -> efi::Status {
    let Some(device) = (unsafe { interface.device.as_ref() }) else { return efi::Status::INVALID_PARAMETER };
    let Some(bus) = (unsafe { device.bus.as_mut() }) else { return efi::Status::INVALID_PARAMETER };
    unsafe { usb_hc_set_root_hub_port_feature(bus, port, feature) }
}

/// Clears a root-hub port feature.
pub unsafe fn usb_root_hub_clear_port_feature(interface: &mut UsbInterface, port: u8, feature: UsbPortFeature) -> efi::Status {
    let Some(device) = (unsafe { interface.device.as_ref() }) else { return efi::Status::INVALID_PARAMETER };
    let Some(bus) = (unsafe { device.bus.as_mut() }) else { return efi::Status::INVALID_PARAMETER };
    unsafe { usb_hc_clear_root_hub_port_feature(bus, port, feature) }
}

/// Resets a root-hub port and waits for reset completion.
pub unsafe fn usb_root_hub_reset_port(
    interface: &mut UsbInterface,
    port: u8,
    timing: &dyn TimingServices,
) -> efi::Status {
    let status = unsafe { usb_root_hub_set_port_feature(interface, port, UsbPortFeature::Reset) };
    if status != efi::Status::SUCCESS {
        return status;
    }

    let _ = timing.stall(Duration::from_micros(USB_SET_ROOT_PORT_RESET_STALL));

    let status = unsafe { usb_root_hub_clear_port_feature(interface, port, UsbPortFeature::Reset) };
    if status != efi::Status::SUCCESS {
        return status;
    }

    let _ = timing.stall(Duration::from_micros(USB_CLR_ROOT_PORT_RESET_STALL));

    let mut port_status = UsbPortStatus { port_status: 0, port_change_status: 0 };
    let mut reset_finished = false;
    for _ in 0..USB_WAIT_PORT_STS_CHANGE_LOOP {
        let status = unsafe { usb_root_hub_get_port_status(interface, port, &mut port_status) };
        if status != efi::Status::SUCCESS {
            return status;
        }
        if port_status.port_status & USB_PORT_STAT_RESET == 0 {
            reset_finished = true;
            break;
        }
        let _ = timing.stall(Duration::from_micros(USB_WAIT_PORT_STS_CHANGE_STALL));
    }

    if !reset_finished {
        return efi::Status::TIMEOUT;
    }

    if port_status.port_status & USB_PORT_STAT_ENABLE == 0 {
        if interface.max_speed == EFI_USB_SPEED_HIGH {
            let _ = unsafe { usb_root_hub_set_port_feature(interface, port, UsbPortFeature::Owner) };
            return efi::Status::NOT_FOUND;
        }

        let status = unsafe { usb_root_hub_set_port_feature(interface, port, UsbPortFeature::Enable) };
        if status != efi::Status::SUCCESS {
            return status;
        }
        let _ = timing.stall(Duration::from_micros(USB_SET_ROOT_PORT_ENABLE_STALL));
    }

    efi::Status::SUCCESS
}

/// Releases root-hub state and closes its timer event.
pub fn usb_root_hub_release(
    interface: &mut UsbInterface,
    events: &dyn EventServices,
    timers: &dyn TimerEventServices,
) -> core::result::Result<(), EfiError> {
    if !interface.hub_notify.is_null() {
        let event = Event::from_raw(interface.hub_notify).ok_or(EfiError::InvalidParameter)?;
        timers.set_timer(event, TimerType::Cancel).map_err(EfiError::from)?;
        events.close_event(event).map_err(EfiError::from)?;
    }
    interface.is_hub = false.into();
    interface.hub_api = core::ptr::null_mut();
    interface.hub_notify = core::ptr::null_mut();
    Ok(())
}
