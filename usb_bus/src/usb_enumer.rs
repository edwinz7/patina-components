//! USB bus enumeration interface translated from `UsbEnumer.h`.

#![allow(dead_code)]

use alloc::boxed::Box;
use core::{ffi::c_void, ptr};
use r_efi::efi;

use crate::usb_2_host_controller::{UsbPortFeature, UsbPortStatus};
use crate::usb_bus_defs::{UsbDevice, UsbInterface, USB_INTERFACE_SIGNATURE, USB_MAX_INTERFACE};
use crate::usb_desc::{usb_free_dev_desc, UsbEndpointDesc, UsbInterfaceDesc};
use crate::usb_io_impl::new_usb_io_protocol;
//use crate::usb_utility::{usb_close_host_proto_by_child, usb_open_host_proto_by_child, usb_get_current_tpl};

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
pub type UsbHubGetPortStatus = unsafe extern "C" fn(*mut UsbInterface, u8, *mut UsbPortStatus) -> efi::Status;

/// Clears a hub port change notification.
pub type UsbHubClearPortChange = unsafe extern "C" fn(*mut UsbInterface, u8);

/// Sets a feature on a hub port.
pub type UsbHubSetPortFeature = unsafe extern "C" fn(*mut UsbInterface, u8, UsbPortFeature) -> efi::Status;

/// Clears a feature on a hub port.
pub type UsbHubClearPortFeature = unsafe extern "C" fn(*mut UsbInterface, u8, UsbPortFeature) -> efi::Status;

/// Resets a hub port.
pub type UsbHubResetPort = unsafe extern "C" fn(*mut UsbInterface, u8) -> efi::Status;

/// Releases a hub interface.
pub type UsbHubRelease = unsafe extern "C" fn(*mut UsbInterface) -> efi::Status;

/// Returns the endpoint descriptor with the requested address.
pub fn usb_get_endpoint_desc(usb_if: &mut UsbInterface, endpoint_address: u8) -> Option<&mut UsbEndpointDesc> {
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
pub fn usb_select_setting(interface: &mut UsbInterfaceDesc, alternate: u8) -> efi::Status {
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

/// Creates an interface object for a parsed interface descriptor.
pub fn usb_create_interface(device: &mut UsbDevice, descriptor: &mut UsbInterfaceDesc) -> Option<Box<UsbInterface>> {
    let setting = unsafe { descriptor.settings[descriptor.active_index].as_mut()? };
    Some(Box::new(UsbInterface {
        signature: USB_INTERFACE_SIGNATURE as usize,
        device: device as *mut UsbDevice,
        if_desc: descriptor as *mut UsbInterfaceDesc,
        if_setting: setting,
        handle: ptr::null_mut(),
        usb_io: new_usb_io_protocol(),
        device_path: ptr::null_mut(),
        is_managed: false.into(),
        is_hub: false.into(),
        hub_api: ptr::null_mut(),
        num_of_port: 0,
        hub_notify: ptr::null_mut(),
        hub_ep: ptr::null_mut(),
        change_map: ptr::null_mut(),
        max_speed: 0,
        poll_count: 0,
    }))
}

/// Releases an interface object and its driver-owned resources.
pub fn usb_free_interface(_interface: Box<UsbInterface>) -> efi::Status {
    efi::Status::SUCCESS
}

/// Creates a child device associated with a hub port.
pub fn usb_create_device(parent_interface: &mut UsbInterface, parent_port: u8) -> Option<Box<UsbDevice>> {
    let parent = unsafe { parent_interface.device.as_ref()? };
    Some(Box::new(UsbDevice {
        bus: parent.bus,
        speed: 0,
        address: 0,
        max_packet0: 8,
        dev_desc: ptr::null_mut(),
        active_config: ptr::null_mut(),
        lang_id: [0; 16],
        total_lang_id: 0,
        num_of_interface: 0,
        interfaces: [ptr::null_mut(); USB_MAX_INTERFACE],
        translator: ptr::null_mut(),
        parent_addr: parent.address,
        parent_if: parent_interface,
        parent_port,
        tier: parent.tier.saturating_add(1),
        connected: true.into(),
        disconnect_fail: false.into(),
    }))
}

/// Frees a device and its parsed descriptor tree.
pub unsafe fn usb_free_device(mut device: Box<UsbDevice>) {
    if !device.dev_desc.is_null() {
        unsafe { usb_free_dev_desc(Box::from_raw(device.dev_desc)) };
        device.dev_desc = ptr::null_mut();
    }
}

/// Connects a device interface. Driver-manager integration is not yet available.
pub fn usb_connect_driver(_interface: &mut UsbInterface) -> efi::Status {
    efi::Status::UNSUPPORTED
}

/// Disconnects a device interface. Driver-manager integration is not yet available.
pub fn usb_disconnect_driver(_interface: &mut UsbInterface) -> efi::Status {
    efi::Status::SUCCESS
}

/// Selects a device configuration and creates its interface objects.
pub unsafe fn usb_select_config(device: &mut UsbDevice, config_value: u8) -> efi::Status {
    let Some(device_descriptor) = (unsafe { device.dev_desc.as_mut() }) else {
        return efi::Status::NOT_FOUND;
    };
    let configuration_count = device_descriptor.descriptor.num_configurations as usize;
    let mut selected = None;
    for index in 0..configuration_count {
        let config = unsafe { *device_descriptor.configs.add(index) };
        if let Some(config) = unsafe { config.as_ref() } {
            if config.descriptor.configuration_value == config_value {
                selected = Some(config);
                break;
            }
        }
    }
    let Some(config) = selected else { return efi::Status::NOT_FOUND };
    device.active_config = config as *const _ as *mut _;

    let interface_count = config.descriptor.num_interfaces as usize;
    for index in 0..interface_count {
        let descriptor = unsafe { &mut **config.interfaces.add(index) };
        if descriptor.num_of_setting == 0 {
            continue;
        }
        let alternate = unsafe { (*descriptor.settings[0]).descriptor.alternate_setting };
        let status = usb_select_setting(descriptor, alternate);
        if status != efi::Status::SUCCESS {
            return status;
        }
        let Some(interface) = usb_create_interface(device, descriptor) else {
            device.num_of_interface = index as u8;
            return efi::Status::OUT_OF_RESOURCES;
        };
        device.interfaces[index] = Box::into_raw(interface);
        device.num_of_interface = (index + 1) as u8;
    }

    efi::Status::SUCCESS
}

/// Removes all interfaces from the active device configuration.
pub unsafe fn usb_remove_config(device: &mut UsbDevice) -> efi::Status {
    let mut result = efi::Status::SUCCESS;
    for index in 0..device.num_of_interface as usize {
        let interface = device.interfaces[index];
        if interface.is_null() {
            continue;
        }
        let status = usb_disconnect_driver(unsafe { &mut *interface });
        if status == efi::Status::SUCCESS {
            drop(unsafe { Box::from_raw(interface) });
            device.interfaces[index] = ptr::null_mut();
        } else {
            result = status;
        }
    }
    device.active_config = ptr::null_mut();
    result
}

/// Finds a child device attached to a hub port.
pub unsafe fn usb_find_child(hub_interface: &UsbInterface, port: u8) -> Option<&mut UsbDevice> {
    let hub_device = unsafe { hub_interface.device.as_ref()? };
    let bus = unsafe { hub_device.bus.as_ref()? };
    let max_devices = (bus.max_devices as usize).min(bus.devices.len());
    for index in 1..max_devices {
        let child = unsafe { bus.devices[index].as_mut()? };
        if child.parent_addr == hub_device.address && child.parent_port == port {
            return Some(child);
        }
    }
    None
}

/// Removes a device and its descendants from the bus.
pub unsafe fn usb_remove_device(device: &mut UsbDevice) -> efi::Status {
    let Some(bus) = (unsafe { device.bus.as_mut() }) else { return efi::Status::INVALID_PARAMETER };
    let max_devices = (bus.max_devices as usize).min(bus.devices.len());
    for index in 1..max_devices {
        let child = bus.devices[index];
        if child.is_null() || unsafe { (*child).parent_addr } != device.address {
            continue;
        }
        let status = unsafe { usb_remove_device(&mut *child) };
        if status == efi::Status::SUCCESS {
            bus.devices[index] = ptr::null_mut();
        } else {
            return status;
        }
    }
    device.connected = false.into();
    unsafe { usb_remove_config(device) }
}

/// Enumerates a newly connected device. Hub protocol integration is pending.
pub fn usb_enumerate_new_device(
    _hub_interface: &mut UsbInterface,
    _port: u8,
    _reset_is_needed: bool,
) -> efi::Status {
    efi::Status::UNSUPPORTED
}

/// Enumerates a changed hub port. Hub protocol integration is pending.
pub fn usb_enumerate_port(_hub_interface: &mut UsbInterface, _port: u8) -> efi::Status {
    efi::Status::UNSUPPORTED
}

/// Handles changed ports for a hub interface.
pub unsafe extern "efiapi" fn usb_hub_enumeration(_event: efi::Event, _context: *mut c_void) {}

/// Handles changed ports for the root hub.
pub unsafe extern "efiapi" fn usb_root_hub_enumeration(_event: efi::Event, _context: *mut c_void) {}
