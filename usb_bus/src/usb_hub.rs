//! USB hub definitions translated from `UsbHub.h`.

#![allow(dead_code)]

extern crate alloc;

use alloc::{vec, vec::Vec};
use core::{ffi::c_void, mem};
use r_efi::efi;
use patina::uefi::{
    boot_services::{BootServices, tpl::Tpl},
    event::{EventTimerType, EventType},
};

use crate::usb_2_host_controller::{
    UsbDataDirection, UsbPortFeature, UsbPortStatus,
    USB_PORT_STAT_C_CONNECTION, USB_PORT_STAT_C_ENABLE,
    USB_PORT_STAT_C_OVERCURRENT, USB_PORT_STAT_C_RESET,
    USB_PORT_STAT_C_SUSPEND, USB_PORT_STAT_RESET,
    EFI_USB_SPEED_SUPER,
};
use crate::usb_bus_defs::{
    UsbDevice, UsbInterface,
    USB_SET_PORT_RECOVERY_STALL, USB_WAIT_PORT_STS_CHANGE_STALL,
    USB_HUB_POLL_INTERVAL, USB_ROOTHUB_POLL_INTERVAL,
};
use crate::usb_desc::usb_ctrl_request;
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
pub unsafe fn usb_hub_ctrl_set_port_feature(hub: &mut UsbDevice, port: u8, feature: UsbPortFeature) -> efi::Status {
    unsafe { usb_ctrl_request(hub, UsbDataDirection::NoData, USB_REQ_TYPE_CLASS, USB_HUB_TARGET_PORT, USB_HUB_REQ_SET_FEATURE as usize, feature as u16, port as u16 + 1, core::ptr::null_mut(), 0) }
}

/// Clears a feature on a hub port. The USB hub port number is one-based.
pub unsafe fn usb_hub_ctrl_clear_port_feature_enum(hub: &mut UsbDevice, port: u8, feature: UsbPortFeature) -> efi::Status {
    unsafe { usb_hub_ctrl_clear_port_feature(hub, port, feature as u16) }
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
    unsafe { usb_hub_ctrl_set_port_feature(device, port, feature) }
}

/// Clears a feature on a normal hub port.
pub unsafe fn usb_hub_clear_port_feature(interface: &mut UsbInterface, port: u8, feature: UsbPortFeature) -> efi::Status {
    let Some(device) = (unsafe { interface.device.as_mut() }) else {
        return efi::Status::INVALID_PARAMETER;
    };
    unsafe { usb_hub_ctrl_clear_port_feature_enum(device, port, feature) }
}

/// Resets a normal hub port and waits for the reset-change indication.
pub unsafe fn usb_hub_reset_port(interface: &mut UsbInterface, port: u8) -> efi::Status {
    unsafe { usb_hub_reset_port_with_stall(interface, port, |_| {}) }
}

/// Resets a normal hub port, using Boot Services when supplied for timing delays.
pub unsafe fn usb_hub_reset_port_with_boot_services<U: BootServices>(
    interface: &mut UsbInterface,
    port: u8,
    boot_services: &U,
) -> efi::Status {
    unsafe { usb_hub_reset_port_with_stall(interface, port, |microseconds| {
        let _ = boot_services.stall(microseconds);
    }) }
}

unsafe fn usb_hub_reset_port_with_stall<F: FnMut(usize)>(
    interface: &mut UsbInterface,
    port: u8,
    mut stall: F,
) -> efi::Status {
    let status = unsafe { usb_hub_set_port_feature(interface, port, UsbPortFeature::Reset) };
    if status != efi::Status::SUCCESS {
        return status;
    }
    for _ in 0..USB_WAIT_PORT_STS_CHANGE_LOOP {
        let mut port_status = UsbPortStatus { port_status: 0, port_change_status: 0 };
        if unsafe { usb_hub_get_port_status(interface, port, &mut port_status) } != efi::Status::SUCCESS {
            return efi::Status::DEVICE_ERROR;
        }
        if port_status.port_change_status & USB_PORT_STAT_C_RESET != 0 {
            stall(USB_SET_PORT_RECOVERY_STALL as usize);
            return efi::Status::SUCCESS;
        }
        stall(USB_WAIT_PORT_STS_CHANGE_STALL as usize);
    }
    efi::Status::TIMEOUT
}

/// Releases normal-hub state from an interface.
pub fn usb_hub_release(interface: &mut UsbInterface) -> efi::Status {
    interface.is_hub = false.into();
    interface.hub_api = core::ptr::null_mut();
    interface.hub_ep = core::ptr::null_mut();
    interface.hub_notify = core::ptr::null_mut();
    efi::Status::SUCCESS
}

/// Releases normal-hub state and closes its Boot Services event.
pub fn usb_hub_release_with_boot_services<U: BootServices>(interface: &mut UsbInterface, boot_services: &U) -> efi::Status {
    if !interface.hub_notify.is_null() {
        if let Err(status) = boot_services.close_event(interface.hub_notify) {
            return status;
        }
    }
    usb_hub_release(interface)
}

/// Initializes the normal-hub state that does not require Boot Services events.
pub unsafe fn usb_hub_init(interface: &mut UsbInterface) -> efi::Status {
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
    interface.is_hub = true.into();
    interface.num_of_port = descriptor[2];
    interface.hub_ep = endpoint as *const _ as *mut _;
    efi::Status::UNSUPPORTED
}

/// Initializes a normal hub and registers its Boot Services event and interrupt poll.
pub unsafe fn usb_hub_init_with_boot_services<U: BootServices + 'static>(
    interface: &mut UsbInterface,
    boot_services: &'static U,
) -> efi::Status {
    let status = unsafe { usb_hub_init(interface) };
    if status != efi::Status::UNSUPPORTED {
        return status;
    }

    let event = match boot_services.create_event(
        EventType::NOTIFY_SIGNAL,
        Tpl::CALLBACK,
        Some(usb_hub_enumeration),
        interface as *mut UsbInterface as *mut c_void,
    ) {
        Ok(event) => event,
        Err(status) => return status,
    };
    interface.hub_notify = event;

    let Some(endpoint) = (unsafe { interface.hub_ep.as_ref() }) else {
        let _ = boot_services.close_event(event);
        interface.hub_notify = core::ptr::null_mut();
        return efi::Status::DEVICE_ERROR;
    };
    let status = unsafe {
        (interface.usb_io.async_interrupt_transfer)(
            &mut interface.usb_io,
            endpoint.descriptor.endpoint_address,
            true.into(),
            USB_HUB_POLL_INTERVAL as usize,
            interface.num_of_port as usize / 8 + 1,
            Some(usb_on_hub_interrupt),
            interface as *mut UsbInterface as *mut c_void,
        )
    };
    if status != efi::Status::SUCCESS {
        let _ = boot_services.close_event(event);
        interface.hub_notify = core::ptr::null_mut();
        interface.is_hub = false.into();
    }
    status
}

/// Queries root-hub capabilities and initializes its port count.
pub unsafe fn usb_root_hub_init(interface: &mut UsbInterface) -> efi::Status {
    let Some(device) = (unsafe { interface.device.as_ref() }) else {
        return efi::Status::INVALID_PARAMETER;
    };
    let Some(bus) = (unsafe { device.bus.as_mut() }) else {
        return efi::Status::INVALID_PARAMETER;
    };
    let mut max_speed = 0;
    let mut num_ports = 0;
    let mut support_64 = 0;
    let status = unsafe { usb_hc_get_capability(bus, &mut max_speed, &mut num_ports, &mut support_64) };
    if status != efi::Status::SUCCESS {
        return status;
    }
    interface.is_hub = true.into();
    interface.max_speed = max_speed;
    interface.num_of_port = num_ports;
    efi::Status::UNSUPPORTED
}

/// Initializes a root hub and configures its periodic Boot Services timer.
pub unsafe fn usb_root_hub_init_with_boot_services<U: BootServices + 'static>(
    interface: &mut UsbInterface,
    boot_services: &'static U,
) -> efi::Status {
    let status = unsafe { usb_root_hub_init(interface) };
    if status != efi::Status::UNSUPPORTED {
        return status;
    }

    let event = match boot_services.create_event(
        EventType::TIMER | EventType::NOTIFY_SIGNAL,
        Tpl::CALLBACK,
        Some(usb_root_hub_enumeration),
        interface as *mut UsbInterface as *mut c_void,
    ) {
        Ok(event) => event,
        Err(status) => return status,
    };
    interface.hub_notify = event;
    if let Err(status) = boot_services.signal_event(event) {
        let _ = boot_services.close_event(event);
        interface.hub_notify = core::ptr::null_mut();
        interface.is_hub = false.into();
        return status;
    }
    if let Err(status) = boot_services.set_timer(
        event,
        EventTimerType::Periodic,
        USB_ROOTHUB_POLL_INTERVAL,
    ) {
        let _ = boot_services.close_event(event);
        interface.hub_notify = core::ptr::null_mut();
        interface.is_hub = false.into();
        return status;
    }
    efi::Status::SUCCESS
}

/// Reads a root-hub port status through the host-controller protocol.
pub unsafe fn usb_root_hub_get_port_status(interface: &mut UsbInterface, port: u8, status: &mut UsbPortStatus) -> efi::Status {
    let Some(device) = (unsafe { interface.device.as_ref() }) else { return efi::Status::INVALID_PARAMETER };
    let Some(bus) = (unsafe { device.bus.as_mut() }) else { return efi::Status::INVALID_PARAMETER };
    unsafe { usb_hc_get_root_hub_port_status(bus, port, status) }
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

/// Releases root-hub state.
pub fn usb_root_hub_release(interface: &mut UsbInterface) -> efi::Status {
    interface.is_hub = false.into();
    interface.hub_api = core::ptr::null_mut();
    interface.hub_notify = core::ptr::null_mut();
    efi::Status::SUCCESS
}

/// Releases root-hub state and closes its Boot Services timer event.
pub fn usb_root_hub_release_with_boot_services<U: BootServices>(
    interface: &mut UsbInterface,
    boot_services: &U,
) -> efi::Status {
    if !interface.hub_notify.is_null() {
        if let Err(status) = boot_services.set_timer(interface.hub_notify, EventTimerType::Cancel, 0) {
            return status;
        }
        if let Err(status) = boot_services.close_event(interface.hub_notify) {
            return status;
        }
    }
    usb_root_hub_release(interface)
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

/// Resets a root-hub port and waits for reset completion.
pub unsafe fn usb_root_hub_reset_port(interface: &mut UsbInterface, port: u8) -> efi::Status {
    let status = unsafe { usb_root_hub_set_port_feature(interface, port, UsbPortFeature::Reset) };
    if status != efi::Status::SUCCESS {
        return status;
    }
    for _ in 0..USB_WAIT_PORT_STS_CHANGE_LOOP {
        let mut port_status = UsbPortStatus { port_status: 0, port_change_status: 0 };
        let status = unsafe { usb_root_hub_get_port_status(interface, port, &mut port_status) };
        if status != efi::Status::SUCCESS {
            return status;
        }
        if port_status.port_change_status & USB_PORT_STAT_C_RESET != 0 {
            let _ = unsafe { usb_root_hub_clear_port_feature(interface, port, UsbPortFeature::ResetChange) };
            if port_status.port_status & USB_PORT_STAT_RESET == 0 {
                return efi::Status::SUCCESS;
            }
        }
    }
    efi::Status::TIMEOUT
}

/// Processes a hub interrupt completion. Event signaling and transfer resubmission
/// require Boot Services integration that is not yet represented in this crate.
pub unsafe extern "efiapi" fn usb_on_hub_interrupt(
    _data: *mut c_void,
    _data_length: usize,
    _context: *mut c_void,
    _result: u32,
) -> efi::Status {
    efi::Status::UNSUPPORTED
}
