//! USB bus enumeration interface translated from `UsbEnumer.h`.

#![allow(dead_code)]

use r_efi::efi;

use crate::usb_bus_defs::{UsbDevice, UsbInterface};
use crate::usb_desc::{UsbEndpointDesc, UsbInterfaceDesc};

#[path = "../../protocols/usb_2_host_controller.rs"]
mod usb_2_host_controller;

use usb_2_host_controller::{UsbPortFeature, UsbPortStatus};

/// Advances a byte and bit position to the next bit.
pub fn usb_next_bit(byte: &mut u8, bit: &mut u8) {
    *bit += 1;
    if *bit > 7 {
        *byte += 1;
        *bit = 0;
    }
}

/// Initializes a hub interface for enumeration.
pub type UsbHubInit = unsafe extern "C" fn(*mut UsbInterface) -> efi::Status;

/// Gets and acknowledges the changed status of a hub port.
pub type UsbHubGetPortStatus =
    unsafe extern "C" fn(*mut UsbInterface, u8, *mut UsbPortStatus) -> efi::Status;

/// Clears a hub port change notification.
pub type UsbHubClearPortChange = unsafe extern "C" fn(*mut UsbInterface, u8);

/// Sets a feature on a hub port.
pub type UsbHubSetPortFeature =
    unsafe extern "C" fn(*mut UsbInterface, u8, UsbPortFeature) -> efi::Status;

/// Clears a feature on a hub port.
pub type UsbHubClearPortFeature =
    unsafe extern "C" fn(*mut UsbInterface, u8, UsbPortFeature) -> efi::Status;

/// Resets a hub port.
pub type UsbHubResetPort = unsafe extern "C" fn(*mut UsbInterface, u8) -> efi::Status;

/// Releases a hub interface.
pub type UsbHubRelease = unsafe extern "C" fn(*mut UsbInterface) -> efi::Status;

/// Returns the endpoint descriptor with the requested address.
pub fn usb_get_endpoint_desc(
    usb_if: &mut UsbInterface,
    endpoint_address: u8,
) -> Option<&mut UsbEndpointDesc> {
    let setting = unsafe { &mut *usb_if.if_setting };
    let endpoint_count = setting.descriptor.num_endpoints;

    for index in 0..endpoint_count {
        let endpoint = unsafe { &mut **setting.endpoints.add(index as usize) };
        if endpoint.descriptor.endpoint_address == endpoint_address {
            return Some(endpoint);
        }
    }

    None
}

/// Selects an alternate setting for an interface.
pub fn usb_select_setting(
    interface: &mut UsbInterfaceDesc,
    alternate: u8,
) -> efi::Status {
    let mut selected_index = None;

    for index in 0..interface.num_of_setting {
        let setting = unsafe { &*interface.settings[index] };
        if setting.descriptor.alternate_setting == alternate {
            selected_index = Some(index);
            break;
        }
    }

    let Some(index) = selected_index else {
        return efi::Status::NOT_FOUND;
    };

    interface.active_index = index;
    let setting = unsafe { &*interface.settings[index] };
    for endpoint_index in 0..setting.descriptor.num_endpoints {
        let endpoint = unsafe { &mut **setting.endpoints.add(endpoint_index as usize) };
        endpoint.toggle = 0;
    }

    efi::Status::SUCCESS
}

unsafe extern "C" {

    /// Selects a configuration for a device.
    pub fn usb_select_config(device: *mut UsbDevice, config_index: u8) -> efi::Status;

    /// Removes the current configuration from a device.
    pub fn usb_remove_config(device: *mut UsbDevice) -> efi::Status;

    /// Removes a device and all of its children from the bus.
    pub fn usb_remove_device(device: *mut UsbDevice) -> efi::Status;
}

unsafe extern "efiapi" {
    /// Enumerates changed ports on a hub.
    pub fn usb_hub_enumeration(event: efi::Event, context: *mut core::ffi::c_void);

    /// Enumerates changed ports on a root hub.
    pub fn usb_root_hub_enumeration(event: efi::Event, context: *mut core::ffi::c_void);
}