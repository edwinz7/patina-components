//! USB descriptor model translated from `UsbDesc.h`.

#![allow(dead_code)]

extern crate alloc;

use alloc::{
    alloc::{Layout, alloc as allocate},
    boxed::Box,
    vec::Vec,
};
use core::{ffi::c_void, mem, ptr, slice, time::Duration};
use r_efi::{
    efi::{Status, protocols::usb_io},
    industry::usb::{ConfigDescriptor, DeviceDescriptor, EndpointDescriptor, InterfaceDescriptor},
};

use crate::usb_2_host_controller::{UsbDataDirection, UsbDeviceRequest};
use crate::usb_bus_defs::{
    USB_CLEAR_FEATURE_REQUEST_TIMEOUT, USB_GENERAL_DEVICE_REQUEST_TIMEOUT,
    USB_RETRY_MAX_PACK_SIZE_STALL, UsbDevice,
};
use crate::usb_utility::usb_hc_control_transfer;

/// Maximum number of alternate settings retained for one USB interface.
pub const USB_MAX_INTERFACE_SETTING: usize = 256;

pub const USB_DESC_TYPE_DEVICE: u8 = 0x01;
pub const USB_DESC_TYPE_CONFIG: u8 = 0x02;
pub const USB_DESC_TYPE_STRING: u8 = 0x03;
pub const USB_DESC_TYPE_INTERFACE: u8 = 0x04;
pub const USB_DESC_TYPE_ENDPOINT: u8 = 0x05;

pub const USB_REQ_TYPE_STANDARD: usize = 0x00;
pub const USB_TARGET_DEVICE: usize = 0x00;
pub const USB_REQ_GET_DESCRIPTOR: usize = 0x06;
pub const USB_REQ_SET_ADDRESS: usize = 0x05;
pub const USB_REQ_SET_CONFIG: usize = 0x09;
pub const USB_REQ_CLEAR_FEATURE: usize = 0x01;

/// USB data direction bit used by the standard request-type encoding.
pub const USB_DATA_IN: u32 = 0x01;

/// Builds the `bmRequestType` byte from direction, request type, and target.
pub const fn usb_request_type(direction_is_in: bool, request_type: usize, target: usize) -> u8 {
    (((if direction_is_in { USB_DATA_IN } else { 0 }) << 7) as usize | request_type | target) as u8
}

/// Common two-byte USB descriptor header.
#[repr(C, packed)]
#[derive(Clone, Copy, Debug, Default)]
pub struct DescriptorHeader {
    pub length: u8,
    pub descriptor_type: u8,
}

/// Endpoint descriptor plus the data toggle used by transfers.
#[repr(C)]
pub struct UsbEndpointDesc {
    pub descriptor: EndpointDescriptor,
    pub toggle: u8,
}

/// Interface alternate setting and its endpoint descriptors.
#[repr(C)]
pub struct UsbInterfaceSetting {
    pub descriptor: InterfaceDescriptor,
    pub endpoints: *mut *mut UsbEndpointDesc,
}

/// All alternate settings belonging to one interface.
#[repr(C)]
pub struct UsbInterfaceDesc {
    pub settings: [*mut UsbInterfaceSetting; USB_MAX_INTERFACE_SETTING],
    pub num_of_setting: usize,
    pub active_index: usize,
}

/// Configuration descriptor and its interface descriptors.
#[repr(C)]
pub struct UsbConfigDesc {
    pub descriptor: ConfigDescriptor,
    pub interfaces: *mut *mut UsbInterfaceDesc,
}

/// Device descriptor and its configuration descriptors.
#[repr(C)]
pub struct UsbDeviceDesc {
    pub descriptor: DeviceDescriptor,
    pub configs: *mut *mut UsbConfigDesc,
}

fn try_box_new<T>(value: T) -> Result<Box<T>, T> {
    let layout = Layout::new::<T>();
    if layout.size() == 0 {
        return Ok(Box::new(value));
    }

    let allocation = unsafe { allocate(layout).cast::<T>() };
    if allocation.is_null() {
        return Err(value);
    }

    unsafe {
        allocation.write(value);
        Ok(Box::from_raw(allocation))
    }
}

fn try_boxed_slice<T: Copy>(length: usize, value: T) -> Option<Box<[T]>> {
    if length == 0 {
        return Some(Vec::new().into_boxed_slice());
    }

    let layout = Layout::array::<T>(length).ok()?;
    let allocation = unsafe { allocate(layout).cast::<T>() };
    if allocation.is_null() {
        return None;
    }

    for index in 0..length {
        unsafe { allocation.add(index).write(value) };
    }
    Some(unsafe { Box::from_raw(slice::from_raw_parts_mut(allocation, length)) })
}

fn try_vec_filled<T: Clone>(length: usize, value: T) -> Option<Vec<T>> {
    let mut values = Vec::new();
    values.try_reserve_exact(length).ok()?;
    values.resize(length, value);
    Some(values)
}

/// Frees an interface setting and all of its endpoint descriptors.
pub unsafe fn usb_free_interface_desc(setting: Box<UsbInterfaceSetting>) {
    let endpoint_count = setting.descriptor.num_endpoints as usize;
    let endpoints = setting.endpoints;
    drop(setting);

    if !endpoints.is_null() {
        let endpoint_slots = unsafe { Box::from_raw(slice::from_raw_parts_mut(endpoints, endpoint_count)) };
        for endpoint in endpoint_slots.into_vec() {
            if !endpoint.is_null() {
                drop(unsafe { Box::from_raw(endpoint) });
            }
        }
    }
}

/// Frees a configuration descriptor and its interface descriptors.
pub unsafe fn usb_free_config_desc(config: Box<UsbConfigDesc>) {
    let interface_count = config.descriptor.num_interfaces as usize;
    let interfaces = config.interfaces;
    drop(config);

    if !interfaces.is_null() {
        let interface_slots = unsafe { Box::from_raw(slice::from_raw_parts_mut(interfaces, interface_count)) };
        for interface in interface_slots.into_vec() {
            if interface.is_null() {
                continue;
            }

            let interface = unsafe { Box::from_raw(interface) };
            for setting_index in 0..interface.num_of_setting {
                let setting = interface.settings[setting_index];
                if !setting.is_null() {
                    unsafe { usb_free_interface_desc(Box::from_raw(setting)) };
                }
            }
        }
    }
}

/// Frees a device descriptor and all parsed configurations.
pub unsafe fn usb_free_dev_desc(descriptor: Box<UsbDeviceDesc>) {
    let config_count = descriptor.descriptor.num_configurations as usize;
    let configs = descriptor.configs;
    drop(descriptor);

    if !configs.is_null() {
        let config_slots = unsafe { Box::from_raw(slice::from_raw_parts_mut(configs, config_count)) };
        for config in config_slots.into_vec() {
            if !config.is_null() {
                unsafe { usb_free_config_desc(Box::from_raw(config)) };
            }
        }
    }
}

/// Returns the first descriptor of `descriptor_type` and the bytes consumed.
pub fn usb_create_desc(descriptor_bytes: &[u8], descriptor_type: u8) -> Option<(Vec<u8>, usize)> {
    let required_size = match descriptor_type {
        USB_DESC_TYPE_DEVICE => mem::size_of::<DeviceDescriptor>(),
        USB_DESC_TYPE_CONFIG => mem::size_of::<ConfigDescriptor>(),
        USB_DESC_TYPE_INTERFACE => mem::size_of::<InterfaceDescriptor>(),
        USB_DESC_TYPE_ENDPOINT => mem::size_of::<EndpointDescriptor>(),
        _ => return None,
    };

    if descriptor_bytes.len() < mem::size_of::<DescriptorHeader>() {
        return None;
    }

    let mut offset = 0;
    while offset + mem::size_of::<DescriptorHeader>() <= descriptor_bytes.len() {
        let length = descriptor_bytes[offset] as usize;
        if length == 0 || offset.checked_add(length)? > descriptor_bytes.len() {
            return None;
        }

        if descriptor_bytes[offset + 1] == descriptor_type {
            if length < required_size {
                return None;
            }
            let mut descriptor = Vec::new();
            descriptor.try_reserve_exact(required_size).ok()?;
            descriptor.extend_from_slice(&descriptor_bytes[offset..offset + required_size]);
            return Some((descriptor, offset + length));
        }

        offset += length;
    }

    None
}

fn read_descriptor<T: Copy>(bytes: &[u8]) -> Option<T> {
    if bytes.len() < mem::size_of::<T>() {
        return None;
    }

    Some(unsafe { ptr::read_unaligned(bytes.as_ptr().cast::<T>()) })
}

/// Parses an interface descriptor and its endpoint descriptors.
pub fn usb_parse_interface_desc(bytes: &[u8]) -> Option<(Box<UsbInterfaceSetting>, usize)> {
    let (interface_bytes, consumed) = usb_create_desc(bytes, USB_DESC_TYPE_INTERFACE)?;
    let descriptor = read_descriptor::<InterfaceDescriptor>(&interface_bytes)?;
    let endpoint_count = descriptor.num_endpoints as usize;
    let mut endpoints = Vec::new();
    endpoints.try_reserve_exact(endpoint_count).ok()?;
    let mut offset = consumed;

    for _ in 0..endpoint_count {
        if offset >= bytes.len() {
            break;
        }
        let (endpoint_bytes, endpoint_consumed) = usb_create_desc(&bytes[offset..], USB_DESC_TYPE_ENDPOINT)?;
        let endpoint_descriptor = read_descriptor::<EndpointDescriptor>(&endpoint_bytes)?;
        endpoints.push(try_box_new(UsbEndpointDesc { descriptor: endpoint_descriptor, toggle: 0 }).ok()?);
        offset += endpoint_consumed;
    }

    let mut setting = try_box_new(UsbInterfaceSetting { descriptor, endpoints: ptr::null_mut() }).ok()?;
    if endpoint_count != 0 {
        let mut endpoint_slots: Box<[*mut UsbEndpointDesc]> =
            try_boxed_slice(endpoint_count, ptr::null_mut())?;
        for (slot, endpoint) in endpoint_slots.iter_mut().zip(endpoints) {
            *slot = Box::into_raw(endpoint);
        }
        setting.endpoints = Box::into_raw(endpoint_slots).cast::<*mut UsbEndpointDesc>();
    }

    Some((setting, offset))
}

/// Parses a configuration descriptor and all interface alternate settings.
pub fn usb_parse_config_desc(bytes: &[u8]) -> Option<Box<UsbConfigDesc>> {
    let (config_bytes, header_consumed) = usb_create_desc(bytes, USB_DESC_TYPE_CONFIG)?;
    let descriptor = read_descriptor::<ConfigDescriptor>(&config_bytes)?;
    let interface_count = descriptor.num_interfaces as usize;
    let mut interfaces = Vec::new();
    interfaces.try_reserve_exact(interface_count).ok()?;

    for _ in 0..interface_count {
        interfaces.push(try_box_new(UsbInterfaceDesc {
            settings: [ptr::null_mut(); USB_MAX_INTERFACE_SETTING],
            num_of_setting: 0,
            active_index: 0,
        }).ok()?);
    }

    let mut config = try_box_new(UsbConfigDesc { descriptor, interfaces: ptr::null_mut() }).ok()?;
    if interface_count != 0 {
        let mut interface_slots: Box<[*mut UsbInterfaceDesc]> =
            try_boxed_slice(interface_count, ptr::null_mut())?;
        for (slot, interface) in interface_slots.iter_mut().zip(interfaces) {
            *slot = Box::into_raw(interface);
        }
        config.interfaces = Box::into_raw(interface_slots).cast::<*mut UsbInterfaceDesc>();
    }

    let total_length = descriptor.total_length as usize;
    let parse_length = total_length.min(bytes.len());
    let mut offset = header_consumed;

    while offset + mem::size_of::<InterfaceDescriptor>() <= parse_length {
        let (setting, consumed) = match usb_parse_interface_desc(&bytes[offset..parse_length]) {
            Some(value) => value,
            None => break,
        };
        let interface_number = setting.descriptor.interface_number as usize;
        if interface_number >= interface_count {
            unsafe { usb_free_interface_desc(setting) };
            unsafe { usb_free_config_desc(config) };
            return None;
        }

        let interface = unsafe { &mut **config.interfaces.add(interface_number) };
        if interface.num_of_setting >= USB_MAX_INTERFACE_SETTING {
            unsafe { usb_free_interface_desc(setting) };
            unsafe { usb_free_config_desc(config) };
            return None;
        }
        interface.settings[interface.num_of_setting] = Box::into_raw(setting);
        interface.num_of_setting += 1;
        offset += consumed;
    }

    Some(config)
}

/// Executes a USB control request for a device.
pub unsafe fn usb_ctrl_request(
    usb_dev: &UsbDevice,
    direction: UsbDataDirection,
    request_type: usize,
    target: usize,
    request: usize,
    value: u16,
    index: u16,
    buffer: *mut c_void,
    length: usize,
) -> Status {
    let bus = usb_dev.bus;
    if bus.is_null() {
        return Status::INVALID_PARAMETER;
    }

    let mut device_request = UsbDeviceRequest {
        request_type: usb_request_type(matches!(direction, UsbDataDirection::In), request_type, target),
        request: request as u8,
        value,
        index,
        length: length as u16,
    };
    let mut transfer_length = length;
    let mut usb_result = 0;

    unsafe {
        usb_hc_control_transfer(
            bus,
            usb_dev.address,
            usb_dev.speed,
            usb_dev.max_packet0 as usize,
            &mut device_request,
            direction,
            buffer,
            &mut transfer_length,
            USB_GENERAL_DEVICE_REQUEST_TIMEOUT as usize,
            ptr::from_ref(&usb_dev.translator).cast_mut(),
            &mut usb_result,
        )
    }
}

/// Retrieves a standard USB descriptor into the caller-provided buffer.
pub unsafe fn usb_ctrl_get_desc(
    usb_dev: &UsbDevice,
    descriptor_type: usize,
    descriptor_index: usize,
    language_id: u16,
    buffer: *mut c_void,
    length: usize,
) -> Status {
    unsafe {
        usb_ctrl_request(
            usb_dev,
            UsbDataDirection::In,
            USB_REQ_TYPE_STANDARD,
            USB_TARGET_DEVICE,
            USB_REQ_GET_DESCRIPTOR,
            ((descriptor_type << 8) | descriptor_index) as u16,
            language_id,
            buffer,
            length,
        )
    }
}

/// Retrieves and stores the device's endpoint-zero packet size.
pub unsafe fn usb_get_max_packet_size0(usb_dev: &mut UsbDevice) -> Status {
    let mut descriptor: DeviceDescriptor = unsafe { mem::zeroed() };

    for _ in 0..3 {
        let status = unsafe {
            usb_ctrl_get_desc(
                usb_dev,
                USB_DESC_TYPE_DEVICE as usize,
                0,
                0,
                (&mut descriptor as *mut DeviceDescriptor).cast(),
                8,
            )
        };
        if !status.is_error() {
            usb_dev.max_packet0 = if descriptor.bcd_usb >= 0x0300 && descriptor.max_packet_size0 == 9 {
                1 << 9
            } else {
                descriptor.max_packet_size0 as u32
            };
            return Status::SUCCESS;
        }

        if let Some(bus) = unsafe { usb_dev.bus.as_ref() } {
            let _ = bus
                .hub_services
                .timing
                .stall(Duration::from_micros(USB_RETRY_MAX_PACK_SIZE_STALL));
        }
    }

    Status::DEVICE_ERROR
}

/// Retrieves and stores the device descriptor.
pub unsafe fn usb_get_dev_desc(usb_dev: &mut UsbDevice) -> Status {
    let Ok(mut descriptor) = try_box_new(UsbDeviceDesc {
        descriptor: unsafe { mem::zeroed() },
        configs: ptr::null_mut(),
    }) else {
        return Status::OUT_OF_RESOURCES;
    };
    let status = unsafe {
        usb_ctrl_get_desc(
            usb_dev,
            USB_DESC_TYPE_DEVICE as usize,
            0,
            0,
            (&mut descriptor.descriptor as *mut DeviceDescriptor).cast(),
            mem::size_of::<DeviceDescriptor>(),
        )
    };

    if !status.is_error() {
        usb_dev.dev_desc = Box::into_raw(descriptor);
    }
    status
}

/// Retrieves a string descriptor as UTF-16 code units.
pub unsafe fn usb_get_one_string(usb_dev: &UsbDevice, index: u8, language_id: u16) -> Option<Vec<u16>> {
    let mut header = [0u8; 2];
    let status = unsafe {
        usb_ctrl_get_desc(
            usb_dev,
            USB_DESC_TYPE_STRING as usize,
            index as usize,
            language_id,
            header.as_mut_ptr().cast(),
            header.len(),
        )
    };
    let length = header[0] as usize;
    if status.is_error() || length < 2 || length % 2 != 0 {
        return None;
    }

    let mut bytes = try_vec_filled(length, 0u8)?;
    let status = unsafe {
        usb_ctrl_get_desc(
            usb_dev,
            USB_DESC_TYPE_STRING as usize,
            index as usize,
            language_id,
            bytes.as_mut_ptr().cast(),
            bytes.len(),
        )
    };
    if status.is_error() {
        return None;
    }

    let character_count = (length - 2) / 2;
    let mut string = Vec::new();
    string.try_reserve_exact(character_count).ok()?;
    string.extend(bytes[2..].chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])));
    Some(string)
}

/// Builds the device's supported language-ID table.
pub unsafe fn usb_build_lang_table(usb_dev: &mut UsbDevice) -> Status {
    let Some(language_ids) = (unsafe { usb_get_one_string(usb_dev, 0, 0) }) else {
        return Status::UNSUPPORTED;
    };
    if language_ids.is_empty() {
        return Status::UNSUPPORTED;
    }

    let count = language_ids.len().min(usb_dev.lang_id.len());
    usb_dev.lang_id[..count].copy_from_slice(&language_ids[..count]);
    usb_dev.total_lang_id = count as u16;
    Status::SUCCESS
}

/// Retrieves a complete configuration descriptor buffer.
pub unsafe fn usb_get_one_config(usb_dev: &mut UsbDevice, index: u8) -> Option<Vec<u8>> {
    let mut descriptor: ConfigDescriptor = unsafe { mem::zeroed() };
    let status = unsafe {
        usb_ctrl_get_desc(
            usb_dev,
            USB_DESC_TYPE_CONFIG as usize,
            index as usize,
            0,
            (&mut descriptor as *mut ConfigDescriptor).cast(),
            8,
        )
    };
    let total_length = descriptor.total_length as usize;
    if status.is_error() || total_length < mem::size_of::<ConfigDescriptor>() {
        return None;
    }

    let mut buffer = try_vec_filled(total_length, 0u8)?;
    let status = unsafe {
        usb_ctrl_get_desc(
            usb_dev,
            USB_DESC_TYPE_CONFIG as usize,
            index as usize,
            0,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
        )
    };
    (!status.is_error()).then_some(buffer)
}

/// Retrieves and parses all descriptors belonging to a device.
pub unsafe fn usb_build_desc_table(usb_dev: &mut UsbDevice) -> Status {
    let status = unsafe { usb_get_dev_desc(usb_dev) };
    if status.is_error() {
        return status;
    }

    let descriptor = unsafe { &mut *usb_dev.dev_desc };
    let configuration_count = descriptor.descriptor.num_configurations as usize;
    if configuration_count == 0 {
        return Status::DEVICE_ERROR;
    }

    let Some(mut configurations): Option<Box<[*mut UsbConfigDesc]>> =
        try_boxed_slice(configuration_count, ptr::null_mut())
    else {
        return Status::OUT_OF_RESOURCES;
    };
    for (index, slot) in configurations.iter_mut().enumerate() {
        let Some(buffer) = (unsafe { usb_get_one_config(usb_dev, index as u8) }) else {
            if index == 0 {
                return Status::DEVICE_ERROR;
            }
            break;
        };
        let Some(config) = usb_parse_config_desc(&buffer) else {
            if index == 0 {
                return Status::DEVICE_ERROR;
            }
            break;
        };
        *slot = Box::into_raw(config);
    }
    descriptor.configs = Box::into_raw(configurations).cast();

    let _ = unsafe { usb_build_lang_table(usb_dev) };
    Status::SUCCESS
}

/// Sets the device address through a standard control request.
pub unsafe fn usb_set_address(usb_dev: &mut UsbDevice, address: u8) -> Status {
    unsafe {
        usb_ctrl_request(
            usb_dev,
            UsbDataDirection::NoData,
            USB_REQ_TYPE_STANDARD,
            USB_TARGET_DEVICE,
            USB_REQ_SET_ADDRESS,
            address as u16,
            0,
            ptr::null_mut(),
            0,
        )
    }
}

/// Sets the device configuration through a standard control request.
pub unsafe fn usb_set_config(usb_dev: &mut UsbDevice, configuration: u8) -> Status {
    unsafe {
        usb_ctrl_request(
            usb_dev,
            UsbDataDirection::NoData,
            USB_REQ_TYPE_STANDARD,
            USB_TARGET_DEVICE,
            USB_REQ_SET_CONFIG,
            configuration as u16,
            0,
            ptr::null_mut(),
            0,
        )
    }
}

/// Clears a USB feature through the USB IO protocol.
pub unsafe fn usb_io_clear_feature(
    usb_io: &mut usb_io::Protocol,
    target: usize,
    feature: u16,
    index: u16,
) -> Status {
    let mut request = usb_io::DeviceRequest {
        request_type: usb_request_type(false, USB_REQ_TYPE_STANDARD, target),
        request: USB_REQ_CLEAR_FEATURE as u8,
        value: feature,
        index,
        length: 0,
    };
    let mut usb_result = 0;

    unsafe {
        (usb_io.control_transfer)(
            usb_io,
            &mut request,
            usb_io::NO_DATA,
            USB_CLEAR_FEATURE_REQUEST_TIMEOUT,
            ptr::null_mut(),
            0,
            &mut usb_result,
        )
    }
}
/// MU_CHANGE [BEGIN] - 291137
/// Refreshes the device and configuration descriptors from the device.
pub unsafe fn usb_update_descriptors(usb_dev: &mut UsbDevice) {
    let mut descriptor: DeviceDescriptor = unsafe { mem::zeroed() };
    let status = unsafe {
        usb_ctrl_get_desc(
            usb_dev,
            USB_DESC_TYPE_DEVICE as usize,
            0,
            0,
            (&mut descriptor as *mut DeviceDescriptor).cast(),
            mem::size_of::<DeviceDescriptor>(),
        )
    };
    if status.is_error() {
        return;
    }

    for index in 0..descriptor.num_configurations {
        let _ = unsafe { usb_get_one_config(usb_dev, index) };
    }
}
// MU_CHANGE [END]

#[cfg(test)]
mod tests {
    use super::*;

    const INTERFACE_WITH_TWO_ENDPOINTS: [u8; 9] = [9, USB_DESC_TYPE_INTERFACE, 0, 0, 2, 0xff, 0, 0, 0];
    const ENDPOINT_ONE: [u8; 7] = [7, USB_DESC_TYPE_ENDPOINT, 0x81, 3, 8, 0, 10];
    const ENDPOINT_TWO: [u8; 7] = [7, USB_DESC_TYPE_ENDPOINT, 0x02, 2, 64, 0, 0];

    #[test]
    fn parses_all_interface_endpoints() {
        let mut bytes = Vec::from(INTERFACE_WITH_TWO_ENDPOINTS);
        bytes.extend_from_slice(&ENDPOINT_ONE);
        bytes.extend_from_slice(&ENDPOINT_TWO);

        let (setting, consumed) = usb_parse_interface_desc(&bytes).expect("interface should parse");

        assert_eq!(consumed, bytes.len());
        let endpoints = unsafe { slice::from_raw_parts(setting.endpoints, 2) };
        assert!(!endpoints[0].is_null());
        assert!(!endpoints[1].is_null());
        unsafe { usb_free_interface_desc(setting) };
    }

    #[test]
    fn preserves_partial_interface_when_endpoint_data_ends() {
        let mut bytes = Vec::from(INTERFACE_WITH_TWO_ENDPOINTS);
        bytes.extend_from_slice(&ENDPOINT_ONE);

        let (setting, consumed) = usb_parse_interface_desc(&bytes).expect("partial interface should parse");

        assert_eq!(consumed, bytes.len());
        let endpoints = unsafe { slice::from_raw_parts(setting.endpoints, 2) };
        assert!(!endpoints[0].is_null());
        assert!(endpoints[1].is_null());
        unsafe { usb_free_interface_desc(setting) };
    }
}
