//! USB utility functions translated from `UsbUtility.c`.
//!
//! The transfer helpers preserve the UEFI USB2 host-controller ABI. Device-path
//! policy is represented with owned Rust storage because the Rust bus model does
//! not expose EDK II's intrusive `LIST_ENTRY` implementation.

#![allow(dead_code)]

extern crate alloc;

use alloc::{vec, vec::Vec};
use core::{ffi::c_void, mem, ptr};

#[path = "../../protocols/device_path_temp.rs"]
mod device_path_temp;

use patina::uefi::boot_services::BootServices;
use r_efi::{base::Boolean, efi};

use crate::usb_2_host_controller::{
    AsyncUsbTransferCallback, Protocol as Usb2HcProtocol, Usb2HcTransactionTranslator, UsbDataDirection,
    UsbDeviceRequest, UsbPortFeature,
};
use crate::usb_bus_defs::{USB_INTERFACE_SIGNATURE, UsbBus, UsbInterface};

pub type Result<T = ()> = core::result::Result<T, efi::Status>;

fn host_controller(bus: *mut UsbBus) -> *mut Usb2HcProtocol {
    // SAFETY: Callers must provide a valid UsbBus allocated by the bus driver.
    unsafe { (*bus).usb2_hc.cast() }
}

pub unsafe fn usb_hc_get_capability(
    bus: *mut UsbBus,
    max_speed: *mut u8,
    num_ports: *mut u8,
    is_64_bit_capable: *mut u8,
) -> efi::Status {
    // SAFETY: The bus and host-controller pointers are owned by the active driver binding.
    unsafe { ((*host_controller(bus)).get_capability)(host_controller(bus), max_speed, num_ports, is_64_bit_capable) }
}

pub unsafe fn usb_hc_get_root_hub_port_status(
    bus: *mut UsbBus,
    port_index: u8,
    port_status: *mut crate::usb_2_host_controller::UsbPortStatus,
) -> efi::Status {
    // SAFETY: The bus and host-controller pointers are owned by the active driver binding.
    unsafe { ((*host_controller(bus)).get_root_hub_port_status)(host_controller(bus), port_index, port_status) }
}

/// Sets a root-hub port feature through the USB2 host-controller protocol.
pub unsafe fn usb_hc_set_root_hub_port_feature(
    bus: *mut UsbBus,
    port_index: u8,
    feature: UsbPortFeature,
) -> efi::Status {
    // SAFETY: The bus and host-controller pointers are owned by the active driver binding.
    unsafe { ((*host_controller(bus)).set_root_hub_port_feature)(host_controller(bus), port_index, feature) }
}

/// Clears a root-hub port feature through the USB2 host-controller protocol.
pub unsafe fn usb_hc_clear_root_hub_port_feature(
    bus: *mut UsbBus,
    port_index: u8,
    feature: UsbPortFeature,
) -> efi::Status {
    // SAFETY: The bus and host-controller pointers are owned by the active driver binding.
    unsafe { ((*host_controller(bus)).clear_root_hub_port_feature)(host_controller(bus), port_index, feature) }
}

/// Executes a USB control transfer through the USB2 host-controller protocol.
pub unsafe fn usb_hc_control_transfer(
    bus: *mut UsbBus,
    device_address: u8,
    device_speed: u8,
    max_packet: usize,
    request: *mut UsbDeviceRequest,
    direction: UsbDataDirection,
    data: *mut c_void,
    data_length: *mut usize,
    timeout: usize,
    translator: *mut Usb2HcTransactionTranslator,
    usb_result: *mut u32,
) -> efi::Status {
    // SAFETY: Arguments are forwarded unchanged to the UEFI protocol.
    unsafe {
        ((*host_controller(bus)).control_transfer)(
            host_controller(bus),
            device_address,
            device_speed,
            max_packet,
            request,
            direction,
            data,
            data_length,
            timeout,
            translator,
            usb_result,
        )
    }
}

/// Executes a USB bulk transfer through the USB2 host-controller protocol.
pub unsafe fn usb_hc_bulk_transfer(
    bus: *mut UsbBus,
    device_address: u8,
    endpoint_address: u8,
    device_speed: u8,
    max_packet: usize,
    buffer_count: u8,
    data: *mut c_void,
    data_length: *mut usize,
    data_toggle: *mut u8,
    timeout: usize,
    translator: *mut Usb2HcTransactionTranslator,
    usb_result: *mut u32,
) -> efi::Status {
    // SAFETY: Arguments are forwarded unchanged to the UEFI protocol.
    unsafe {
        ((*host_controller(bus)).bulk_transfer)(
            host_controller(bus),
            device_address,
            endpoint_address,
            device_speed,
            max_packet,
            buffer_count,
            data,
            data_length,
            data_toggle,
            timeout,
            translator,
            usb_result,
        )
    }
}

/// Queues or cancels an asynchronous interrupt transfer.
pub unsafe fn usb_hc_async_interrupt_transfer(
    bus: *mut UsbBus,
    device_address: u8,
    endpoint_address: u8,
    device_speed: u8,
    max_packet: usize,
    is_new_transfer: Boolean,
    data_toggle: *mut u8,
    polling_interval: usize,
    data_length: usize,
    translator: *mut Usb2HcTransactionTranslator,
    callback: Option<AsyncUsbTransferCallback>,
    context: *mut c_void,
) -> efi::Status {
    // SAFETY: Arguments are forwarded unchanged to the UEFI protocol.
    unsafe {
        ((*host_controller(bus)).async_interrupt_transfer)(
            host_controller(bus),
            device_address,
            endpoint_address,
            device_speed,
            max_packet,
            is_new_transfer,
            data_toggle,
            polling_interval,
            data_length,
            translator,
            callback,
            context,
        )
    }
}

/// Executes a synchronous interrupt transfer.
pub unsafe fn usb_hc_sync_interrupt_transfer(
    bus: *mut UsbBus,
    device_address: u8,
    endpoint_address: u8,
    device_speed: u8,
    max_packet: usize,
    data: *mut c_void,
    data_length: *mut usize,
    data_toggle: *mut u8,
    timeout: usize,
    translator: *mut Usb2HcTransactionTranslator,
    usb_result: *mut u32,
) -> efi::Status {
    // SAFETY: Arguments are forwarded unchanged to the UEFI protocol.
    unsafe {
        ((*host_controller(bus)).sync_interrupt_transfer)(
            host_controller(bus),
            device_address,
            endpoint_address,
            device_speed,
            max_packet,
            data,
            data_length,
            data_toggle,
            timeout,
            translator,
            usb_result,
        )
    }
}

/// Opens the host-controller protocol for a child controller.
pub unsafe fn usb_open_host_proto_by_child<U: BootServices + 'static>(
    boot_services: &'static U,
    bus: *mut UsbBus,
    agent: efi::Handle,
    child: efi::Handle,
) -> Result {
    // SAFETY: The bus pointer is valid for the lifetime of the binding.
    let host_handle = unsafe { (*bus).host_handle };
    // SAFETY: Protocol ownership and handles are controlled by Boot Services.
    unsafe {
        boot_services
            .open_protocol::<Usb2HcProtocol>(host_handle, agent, child, efi::OPEN_PROTOCOL_BY_CHILD_CONTROLLER)
            .map(|_| ())
    }
}

/// Closes a host-controller protocol opened for a child controller.
pub fn usb_close_host_proto_by_child<U: BootServices>(
    boot_services: &U,
    bus: *mut UsbBus,
    agent: efi::Handle,
    child: efi::Handle,
) -> Result {
    // SAFETY: The bus pointer is valid for the lifetime of the binding.
    let host_handle = unsafe { (*bus).host_handle };
    boot_services.close_protocol(host_handle, &crate::usb_2_host_controller::PROTOCOL_GUID, agent, child)
}

/// Returns the current task priority level.
///
/// Raising and restoring TPL requires a Boot Services implementation, so this
/// helper is expressed as a caller-provided operation in Rust.
pub fn usb_get_current_tpl<U: BootServices>(boot_services: &U, current_tpl: efi::Tpl) -> efi::Tpl {
    let _ = boot_services;
    current_tpl
}

fn is_end_node(node: &device_path_temp::Protocol) -> bool {
    node.r#type == device_path_temp::TYPE_END && node.sub_type == device_path_temp::END_ENTIRE_DEVICE_PATH_SUBTYPE
}

fn is_usb_node(node: &device_path_temp::Protocol) -> bool {
    node.r#type == device_path_temp::TYPE_MESSAGING
        && matches!(
            node.sub_type,
            device_path_temp::MSG_USB_DP | device_path_temp::MSG_USB_CLASS_DP | device_path_temp::MSG_USB_WWID_DP
        )
}

fn node_length(node: &device_path_temp::Protocol) -> usize {
    u16::from_le_bytes(node.length) as usize
}

fn device_path_temp_size(path: *const device_path_temp::Protocol) -> Option<usize> {
    if path.is_null() {
        return None;
    }

    let mut offset = 0;
    loop {
        // SAFETY: The caller provides a valid, NUL-terminated UEFI device path.
        let node = unsafe { &*path.byte_add(offset) };
        let length = node_length(node);
        if length < mem::size_of::<device_path_temp::Protocol>() {
            return None;
        }
        offset = offset.checked_add(length)?;
        if is_end_node(node) {
            return Some(offset);
        }
    }
}

/// Copies the first contiguous USB portion of a full device path.
pub fn get_usb_dp_from_full_dp(path: *const device_path_temp::Protocol) -> Option<Vec<u8>> {
    let total_size = device_path_temp_size(path)?;
    let mut begin = 0;
    while begin < total_size {
        // SAFETY: `begin` is bounded by the validated path size.
        let node = unsafe { &*path.byte_add(begin) };
        if is_end_node(node) || is_usb_node(node) {
            break;
        }
        begin += node_length(node);
    }

    let mut end = begin;
    while end < total_size {
        // SAFETY: `end` is bounded by the validated path size.
        let node = unsafe { &*path.byte_add(end) };
        if !is_usb_node(node) {
            break;
        }
        end += node_length(node);
    }
    if end == begin {
        return None;
    }

    let mut result = vec![0; end - begin + mem::size_of::<device_path_temp::Protocol>()];
    // SAFETY: The source range is within the validated device path.
    unsafe { ptr::copy_nonoverlapping(path.cast::<u8>().add(begin), result.as_mut_ptr(), end - begin) };
    let end_node = device_path_temp::Protocol {
        r#type: device_path_temp::TYPE_END,
        sub_type: device_path_temp::END_ENTIRE_DEVICE_PATH_SUBTYPE,
        length: (mem::size_of::<device_path_temp::Protocol>() as u16).to_le_bytes(),
    };
    result[end - begin..].copy_from_slice(unsafe {
        core::slice::from_raw_parts(
            (&end_node as *const device_path_temp::Protocol).cast::<u8>(),
            mem::size_of_val(&end_node),
        )
    });
    Some(result)
}

/// Owned replacement for the C `DEVICE_PATH_LIST_ITEM` intrusive list.
#[derive(Default)]
pub struct UsbDevicePathList {
    paths: Vec<Vec<u8>>,
}

/// Searches the list for an identical device path.
pub fn search_usb_dp_in_list(usb_dp: &[u8], list: Option<&UsbDevicePathList>) -> bool {
    list.is_some_and(|list| list.paths.iter().any(|path| path == usb_dp))
}

/// Adds a device path to the list unless it is already present.
pub fn add_usb_dp_to_list(usb_dp: &[u8], list: Option<&mut UsbDevicePathList>) -> Result {
    let Some(list) = list else { return Err(efi::Status::INVALID_PARAMETER) };
    if !search_usb_dp_in_list(usb_dp, Some(list)) {
        list.paths.push(usb_dp.to_vec());
    }
    Ok(())
}

/// Frees all paths in the list.
pub fn usb_bus_free_usb_dp_list(list: Option<&mut UsbDevicePathList>) -> Result {
    let Some(list) = list else { return Err(efi::Status::INVALID_PARAMETER) };
    list.paths.clear();
    Ok(())
}

/// USB class short-form device path fields.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UsbClassDevicePath {
    pub vendor_id: u16,
    pub product_id: u16,
    pub device_class: u8,
    pub device_sub_class: u8,
    pub device_protocol: u8,
}

/// USB WWID short-form device path fields.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UsbWwidDevicePath {
    pub vendor_id: u16,
    pub product_id: u16,
    pub interface_number: u8,
    pub serial_number: Vec<u16>,
}

/// Descriptor values needed by the C class and WWID matching policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UsbInterfaceMatchInfo {
    pub vendor_id: u16,
    pub product_id: u16,
    pub device_class: u8,
    pub device_sub_class: u8,
    pub device_protocol: u8,
    pub interface_number: u8,
    pub interface_class: u8,
    pub interface_sub_class: u8,
    pub interface_protocol: u8,
    pub is_hub: bool,
}

/// Matches a USB class device path against descriptor values.
pub fn match_usb_class(path: &UsbClassDevicePath, interface: &UsbInterfaceMatchInfo) -> bool {
    if interface.is_hub {
        return true;
    }
    (path.vendor_id == 0xffff || path.vendor_id == interface.vendor_id)
        && (path.product_id == 0xffff || path.product_id == interface.product_id)
        && if interface.device_class == 0 {
            (path.device_class == interface.interface_class || path.device_class == 0xff)
                && (path.device_sub_class == interface.interface_sub_class || path.device_sub_class == 0xff)
                && (path.device_protocol == interface.interface_protocol || path.device_protocol == 0xff)
        } else {
            (path.device_class == interface.device_class || path.device_class == 0xff)
                && (path.device_sub_class == interface.device_sub_class || path.device_sub_class == 0xff)
                && (path.device_protocol == interface.device_protocol || path.device_protocol == 0xff)
        }
}

/// Matches the WWID path against a serial number suffix and descriptor values.
pub fn match_usb_wwid(path: &UsbWwidDevicePath, interface: &UsbInterfaceMatchInfo, serial_number: &[u16]) -> bool {
    if interface.is_hub
        || path.vendor_id != interface.vendor_id
        || path.product_id != interface.product_id
        || path.interface_number != interface.interface_number
    {
        return interface.is_hub;
    }
    let wanted = path.serial_number.strip_suffix(&[0]).unwrap_or(&path.serial_number);
    serial_number.len() >= wanted.len() && serial_number[serial_number.len() - wanted.len()..] == *wanted
}

/// Indicates whether an interface is selected by the bus's wanted-path policy.
pub fn usb_bus_is_wanted_usb_io(
    interface: Option<&UsbInterfaceMatchInfo>,
    wanted_paths: &UsbDevicePathList,
    interface_path: &[u8],
) -> bool {
    let Some(interface) = interface else { return false };
    if interface.is_hub {
        return true;
    }
    wanted_paths.paths.iter().any(|path| path == interface_path)
}

/// Connects wanted child controllers. The full handle enumeration is supplied by the caller.
pub fn usb_bus_recursively_connect_wanted_usb_io<U: BootServices>(
    _boot_services: &U,
    _bus: Option<*mut UsbBus>,
    _interfaces: &mut [UsbInterface],
) -> Result {
    // `BootServices::locate_handle_buffer` and the child relationship helpers
    // are not part of the current Patina BootServices trait.
    Err(efi::Status::UNSUPPORTED)
}

/// Confirms that a raw interface pointer has the expected USB interface signature.
pub unsafe fn is_usb_interface(interface: *const UsbInterface) -> bool {
    !interface.is_null() && unsafe { (*interface).signature == USB_INTERFACE_SIGNATURE as usize }
}
