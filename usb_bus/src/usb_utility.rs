//! USB utility functions translated from `UsbUtility.c`.
//!
//! The transfer helpers preserve the UEFI USB2 host-controller ABI. Device-path
//! policy is represented with owned Rust storage because the Rust bus model does
//! not expose EDK II's intrusive `LIST_ENTRY` implementation.

#![allow(dead_code)]

extern crate alloc;

use alloc::{boxed::Box, vec, vec::Vec};
use core::{ffi::c_void, mem, ptr};

use patina::{
    component::service::uefi_services::{
        handle::Handle,
        driver_model::driver::DriverServices,
        protocol::{OpenAttributes, ProtocolError, ProtocolServices, ProtocolServicesExt},
        tpl::{Tpl, PreviousTpl, TplServices},
    },
    protocol::ProtocolInterface,
    BinaryGuid,
};
use r_efi::{base::Boolean, efi};

use crate::usb_2_host_controller::{
    AsyncUsbTransferCallback, Usb2HcProtocol, Usb2HcTransactionTranslator, UsbDataDirection,
    UsbDeviceRequest, UsbPortFeature,
};
use crate::device_path_temp::{
    self, MSG_USB_CLASS_DP, MSG_USB_WWID_DP, MSG_USB_WWID_SERIAL_NUMBER_OFFSET, TYPE_MESSAGING, UsbClassDevicePath,
    UsbWwidDevicePath, EfiDevicePathProtocol,
};
use crate::usb_bus_defs::{
    EfiUsbBusProtocol, UsbBus, UsbDevicePathList, UsbInterface, usb_bus_from_this, usb_bus_from_this_mut,
    usb_interface_from_usb_io,
};
use crate::usb_desc::{USB_MAX_INTERFACE_SETTING, usb_get_one_string};
use crate::usb_hub::{USB_HUB_CLASS_CODE, USB_HUB_SUBCLASS_CODE};

pub type Result<T = ()> = core::result::Result<T, efi::Status>;
pub type ProtocolResult<T = ()> = core::result::Result<T, ProtocolError>;

#[repr(transparent)]
pub(crate) struct UsbIoProtocol(pub(crate) efi::protocols::usb_io::Protocol);

// SAFETY: This transparent wrapper binds the standard USB I/O protocol layout to its UEFI GUID.
unsafe impl ProtocolInterface for UsbIoProtocol {
    const PROTOCOL_GUID: BinaryGuid = BinaryGuid::from_string("2B2F68D6-0CD2-44CF-8E8B-BBA20B1B5B75");
}

fn host_controller(bus: *mut UsbBus) -> *mut Usb2HcProtocol {
    // SAFETY: Callers must provide a valid UsbBus allocated by the bus driver.
    unsafe { (*bus).usb2_hc.cast() }
}

fn is_end_node(node: &EfiDevicePathProtocol) -> bool {
    node.r#type == device_path_temp::TYPE_END && node.sub_type == device_path_temp::END_ENTIRE_DEVICE_PATH_SUBTYPE
}

fn is_usb_node(node: &EfiDevicePathProtocol) -> bool {
    node.r#type == device_path_temp::TYPE_MESSAGING
        && matches!(
            node.sub_type,
            device_path_temp::MSG_USB_DP | device_path_temp::MSG_USB_CLASS_DP | device_path_temp::MSG_USB_WWID_DP
        )
}

fn node_length(node: &EfiDevicePathProtocol) -> usize {
    u16::from_le_bytes(node.length) as usize
}

unsafe fn device_path_temp_size(path: &EfiDevicePathProtocol) -> Option<usize> {
    let path_ptr = ptr::from_ref(path);
    let mut offset = 0;
    loop {
        // SAFETY: The caller provides a valid, NUL-terminated UEFI device path.
        let node = unsafe { &*path_ptr.byte_add(offset) };
        let length = node_length(node);
        if length < mem::size_of::<EfiDevicePathProtocol>() {
            return None;
        }
        offset = offset.checked_add(length)?;
        if is_end_node(node) {
            return Some(offset);
        }
    }
}

/// Releases an owned, end-terminated device path allocation.
pub unsafe fn usb_free_device_path(path: &mut EfiDevicePathProtocol) -> Result {
    let Some(size) = (unsafe { device_path_temp_size(path) }) else {
        return Err(efi::Status::INVALID_PARAMETER);
    };
    let allocation = ptr::slice_from_raw_parts_mut(ptr::from_mut(path).cast::<u8>(), size);
    drop(unsafe { Box::from_raw(allocation) });
    Ok(())
}

fn device_path_node_from_bytes(path: &[u8]) -> Option<EfiDevicePathProtocol> {
    if path.len() < mem::size_of::<EfiDevicePathProtocol>() {
        return None;
    }

    // SAFETY: The slice contains enough bytes, and unaligned reads support packed device-path data.
    Some(unsafe { ptr::read_unaligned(path.as_ptr().cast::<EfiDevicePathProtocol>()) })
}

fn is_all_usb_class_device_path(path: &[u8]) -> bool {
    let class_size = mem::size_of::<UsbClassDevicePath>();
    if path.len() != class_size + mem::size_of::<EfiDevicePathProtocol>() {
        return false;
    }

    // SAFETY: The length check covers both packed records, and unaligned reads support byte storage.
    let class_path = unsafe { ptr::read_unaligned(path.as_ptr().cast::<UsbClassDevicePath>()) };
    let end_node = unsafe {
        ptr::read_unaligned(
            path.as_ptr()
                .add(class_size)
                .cast::<EfiDevicePathProtocol>(),
        )
    };

    class_path.header.r#type == TYPE_MESSAGING
        && class_path.header.sub_type == MSG_USB_CLASS_DP
        && node_length(&class_path.header) == class_size
        && class_path.vendor_id == 0xffff
        && class_path.product_id == 0xffff
        && class_path.device_class == 0xff
        && class_path.device_sub_class == 0xff
        && class_path.device_protocol == 0xff
        && is_end_node(&end_node)
        && node_length(&end_node) == mem::size_of::<EfiDevicePathProtocol>()
}

fn wwid_serial_length(node_length: usize) -> Option<usize> {
    let serial_bytes = node_length.checked_sub(MSG_USB_WWID_SERIAL_NUMBER_OFFSET)?;
    if serial_bytes == 0 || serial_bytes % mem::size_of::<u16>() != 0 {
        return None;
    }

    Some(serial_bytes / mem::size_of::<u16>())
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
    data: *mut *mut c_void,
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
pub fn usb_open_host_proto_by_child(
    protocols: &dyn ProtocolServices,
    bus: *mut UsbBus,
    agent: efi::Handle,
    child: efi::Handle,
) -> ProtocolResult {
    let host_handle = unsafe { (*bus).host_handle };
    let Some(host_handle) = Handle::from_raw(host_handle) else {
        return Err(ProtocolError::InvalidParameter);
    };
    let Some(agent) = Handle::from_raw(agent) else {
        return Err(ProtocolError::InvalidParameter);
    };
    let Some(child) = Handle::from_raw(child) else {
        return Err(ProtocolError::InvalidParameter);
    };
    protocols
        .open_interface(
            host_handle,
            Usb2HcProtocol::PROTOCOL_GUID,
            agent,
            OpenAttributes::ByChildController { controller: child },
        )
        .map(|_| ())
}

/// Closes a host-controller protocol opened for a child controller.
pub fn usb_close_host_proto_by_child(
    protocols: &dyn ProtocolServices,
    bus: *mut UsbBus,
    agent: efi::Handle,
    child: efi::Handle,
) -> ProtocolResult {
    let host_handle = unsafe { (*bus).host_handle };
    let Some(host_handle) = Handle::from_raw(host_handle) else {
        return Err(ProtocolError::InvalidParameter);
    };
    let Some(agent) = Handle::from_raw(agent) else {
        return Err(ProtocolError::InvalidParameter);
    };
    let Some(child) = Handle::from_raw(child) else {
        return Err(ProtocolError::InvalidParameter);
    };
    protocols.close_interface(
        host_handle,
        Usb2HcProtocol::PROTOCOL_GUID,
        agent,
        Some(child),
    )
}

/// Returns the current task priority level, copied from the EDKII glue lib.
pub fn usb_get_current_tpl(tpl: &dyn TplServices) -> PreviousTpl {
    let current_tpl = tpl.raise_tpl(Tpl::HighLevel);
    tpl.restore_tpl(current_tpl);
    current_tpl
}

/// Copies the first contiguous USB portion of a full device path.
///
/// # Safety
///
/// `path` must be the first node of a complete, valid, end-terminated device path.
pub unsafe fn get_usb_dp_from_full_dp(path: &EfiDevicePathProtocol) -> Option<Vec<u8>> {
    let total_size = unsafe { device_path_temp_size(path) }?;
    let path_ptr = ptr::from_ref(path);
    let mut begin = 0;
    while begin < total_size {
        // SAFETY: `begin` is bounded by the validated path size.
        let node = unsafe { &*path_ptr.byte_add(begin) };
        if is_end_node(node) || is_usb_node(node) {
            break;
        }
        begin += node_length(node);
    }

    let mut end = begin;
    while end < total_size {
        // SAFETY: `end` is bounded by the validated path size.
        let node = unsafe { &*path_ptr.byte_add(end) };
        if !is_usb_node(node) {
            break;
        }
        end += node_length(node);
    }
    if end == begin {
        return None;
    }

    let mut result = vec![0; end - begin + mem::size_of::<EfiDevicePathProtocol>()];
    // SAFETY: The source range is within the validated device path.
    unsafe { ptr::copy_nonoverlapping(path_ptr.cast::<u8>().add(begin), result.as_mut_ptr(), end - begin) };
    let end_node = EfiDevicePathProtocol {
        r#type: device_path_temp::TYPE_END,
        sub_type: device_path_temp::END_ENTIRE_DEVICE_PATH_SUBTYPE,
        length: (mem::size_of::<EfiDevicePathProtocol>() as u16).to_le_bytes(),
    };
    result[end - begin..].copy_from_slice(unsafe {
        core::slice::from_raw_parts(
            ptr::from_ref(&end_node).cast::<u8>(),
            mem::size_of_val(&end_node),
        )
    });
    Some(result)
}

/// Searches the list for an identical device path.
pub unsafe fn search_usb_dp_in_list(usb_dp: &EfiDevicePathProtocol, list: Option<&UsbDevicePathList>) -> bool {
    let Some(list) = list else { return false };
    let Some(size) = (unsafe { device_path_temp_size(usb_dp) }) else { return false };
    let usb_dp_ptr = ptr::from_ref(usb_dp);
    // SAFETY: The caller guarantees `usb_dp` points to a complete device path, and its size was validated above.
    list.contains(unsafe { core::slice::from_raw_parts(usb_dp_ptr.cast::<u8>(), size) })
}

/// Adds a device path to the list unless it is already present.
pub unsafe fn add_usb_dp_to_list(usb_dp: &EfiDevicePathProtocol, list: Option<&mut UsbDevicePathList>) -> Result {
    let Some(list) = list else { return Err(efi::Status::INVALID_PARAMETER) };
    let Some(size) = (unsafe { device_path_temp_size(usb_dp) }) else { return Err(efi::Status::INVALID_PARAMETER) };
    let usb_dp_ptr = ptr::from_ref(usb_dp);
    // SAFETY: The caller guarantees `usb_dp` points to a complete device path, and its size was validated above.
    let usb_dp_bytes = unsafe { core::slice::from_raw_parts(usb_dp_ptr.cast::<u8>(), size) };
    if !unsafe { search_usb_dp_in_list(usb_dp, Some(list)) } {
        list.insert_tail_list(usb_dp_bytes.to_vec());
    }
    Ok(())
}

/// Matches a USB class device path against descriptor values.
pub unsafe fn match_usb_class(path: &UsbClassDevicePath, interface: &UsbInterface) -> bool {
    if path.header.r#type != TYPE_MESSAGING || path.header.sub_type != MSG_USB_CLASS_DP {
        return false;
    }
    let Some(interface_desc) = (unsafe { interface.if_desc.as_ref() }) else { return false };
    if interface_desc.active_index >= USB_MAX_INTERFACE_SETTING {
        return false;
    }
    let Some(active_setting) = (unsafe { interface_desc.settings[interface_desc.active_index].as_ref() }) else {
        return false;
    };
    let Some(device) = (unsafe { interface.device.as_ref() }) else { return false };
    let Some(device_desc) = (unsafe { device.dev_desc.as_ref() }) else { return false };
    let active_desc = active_setting.descriptor;
    let device_desc = device_desc.descriptor;

    if active_desc.interface_class == USB_HUB_CLASS_CODE
        && active_desc.interface_sub_class == USB_HUB_SUBCLASS_CODE
    {
        return true;
    }

    (path.vendor_id == 0xffff || path.vendor_id == device_desc.id_vendor)
        && (path.product_id == 0xffff || path.product_id == device_desc.id_product)
        && if device_desc.device_class == 0 {
            (path.device_class == active_desc.interface_class || path.device_class == 0xff)
                && (path.device_sub_class == active_desc.interface_sub_class || path.device_sub_class == 0xff)
                && (path.device_protocol == active_desc.interface_protocol || path.device_protocol == 0xff)
        } else {
            (path.device_class == device_desc.device_class || path.device_class == 0xff)
                && (path.device_sub_class == device_desc.device_sub_class || path.device_sub_class == 0xff)
                && (path.device_protocol == device_desc.device_protocol || path.device_protocol == 0xff)
        }
}

/// Matches the WWID path against a serial number suffix and descriptor values.
pub unsafe fn match_usb_wwid(path: &UsbWwidDevicePath, interface: &UsbInterface) -> bool {
    if path.header.r#type != TYPE_MESSAGING || path.header.sub_type != MSG_USB_WWID_DP {
        return false;
    }
    let Some(interface_desc) = (unsafe { interface.if_desc.as_ref() }) else { return false };
    if interface_desc.active_index >= USB_MAX_INTERFACE_SETTING {
        return false;
    }
    let Some(active_setting) = (unsafe { interface_desc.settings[interface_desc.active_index].as_ref() }) else {
        return false;
    };
    let Some(device) = (unsafe { interface.device.as_ref() }) else { return false };
    let Some(device_desc) = (unsafe { device.dev_desc.as_ref() }) else { return false };
    let active_desc = active_setting.descriptor;
    let device_desc = device_desc.descriptor;

    if active_desc.interface_class == USB_HUB_CLASS_CODE
        && active_desc.interface_sub_class == USB_HUB_SUBCLASS_CODE
    {
        return true;
    }
    if path.vendor_id != device_desc.id_vendor
        || path.product_id != device_desc.id_product
        || path.interface_number != active_desc.interface_number as u16
        || device_desc.str_serial_number == 0
    {
        return false;
    }

    let node_length = u16::from_le_bytes(path.header.length) as usize;
    let Some(serial_length) = wwid_serial_length(node_length) else { return false };
    let serial_ptr = unsafe {
        ptr::from_ref(path)
            .cast::<u8>()
            .add(MSG_USB_WWID_SERIAL_NUMBER_OFFSET)
            .cast::<u16>()
    };
    let mut wanted_serial = Vec::with_capacity(serial_length);
    for index in 0..serial_length {
        wanted_serial.push(u16::from_le(unsafe { ptr::read_unaligned(serial_ptr.add(index)) }));
    }
    if wanted_serial.last() == Some(&0) {
        wanted_serial.pop();
    }

    let language_count = usize::min(device.total_lang_id as usize, device.lang_id.len());
    for index in 0..language_count {
        let language_id = device.lang_id[index];
        let Some(serial_number) = (unsafe { usb_get_one_string(device, device_desc.str_serial_number, language_id) })
        else {
            continue;
        };
        if serial_number.ends_with(&wanted_serial) {
            return true;
        }
    }

    false
}

/// Frees all paths in the list.
pub fn usb_bus_free_usb_dp_list(list: Option<&mut UsbDevicePathList>) -> core::result::Result<(), ProtocolError> {
    let Some(list) = list else { return Err(ProtocolError::InvalidParameter) };
    list.initialize_list_head();
    Ok(())
}

/// Records the USB portion of a remaining device path in the bus policy.
///
/// `None` selects all USB devices, matching the UEFI driver-binding convention.
///
/// # Safety
///
/// When present, `remaining_device_path` must be the first node of a complete,
/// valid, end-terminated device path.
pub unsafe fn usb_bus_add_wanted_usb_io_dp(
    usb_bus_id: &mut EfiUsbBusProtocol,
    remaining_device_path: Option<&EfiDevicePathProtocol>,
) -> core::result::Result<(), ProtocolError> {
    if let Some(path) = remaining_device_path {
        if !is_end_node(path) && !is_usb_node(path) {
            return Err(ProtocolError::InvalidParameter);
        }
    }

    // SAFETY: The caller supplies the private protocol embedded in an exclusively borrowed bus.
    let Some(bus) = (unsafe { usb_bus_from_this_mut(usb_bus_id) }) else {
        return Err(ProtocolError::InvalidParameter);
    };

    let device_path = match remaining_device_path {
        None => {
            bus.wanted_usb_io_dp_list.initialize_list_head();

            let class_node = UsbClassDevicePath {
                header: EfiDevicePathProtocol {
                    r#type: TYPE_MESSAGING,
                    sub_type: MSG_USB_CLASS_DP,
                    length: (mem::size_of::<UsbClassDevicePath>() as u16).to_le_bytes(),
                },
                vendor_id: 0xffff,
                product_id: 0xffff,
                device_class: 0xff,
                device_sub_class: 0xff,
                device_protocol: 0xff,
            };
            let end_node = EfiDevicePathProtocol {
                r#type: device_path_temp::TYPE_END,
                sub_type: device_path_temp::END_ENTIRE_DEVICE_PATH_SUBTYPE,
                length: (mem::size_of::<EfiDevicePathProtocol>() as u16).to_le_bytes(),
            };
            let mut path = Vec::with_capacity(mem::size_of_val(&class_node) + mem::size_of_val(&end_node));
            // SAFETY: Both packed records are copied byte-for-byte into owned storage.
            path.extend_from_slice(unsafe {
                core::slice::from_raw_parts(ptr::from_ref(&class_node).cast::<u8>(), mem::size_of_val(&class_node))
            });
            path.extend_from_slice(unsafe {
                core::slice::from_raw_parts(ptr::from_ref(&end_node).cast::<u8>(), mem::size_of_val(&end_node))
            });
            path
        }
        Some(path) if is_end_node(path) => return Ok(()),
        Some(path) => unsafe { get_usb_dp_from_full_dp(path) }.ok_or(ProtocolError::InvalidParameter)?,
    };

    if !bus.wanted_usb_io_dp_list.contains(&device_path) {
        bus.wanted_usb_io_dp_list.insert_tail_list(device_path);
    }
    Ok(())
}

/// Indicates whether an interface is selected by the bus's wanted-path policy.
pub fn usb_bus_is_wanted_usb_io(bus: &UsbBus, interface: &UsbInterface) -> bool {
    if interface.is_hub {
        return true;
    }

    if bus.wanted_usb_io_dp_list.iter().any(is_all_usb_class_device_path) {
        return true;
    }

    // SAFETY: The interface owns a complete device path for its lifetime.
    let Some(interface_path) = (unsafe { interface.device_path.cast::<EfiDevicePathProtocol>().as_ref() })
        .and_then(|device_path| unsafe { get_usb_dp_from_full_dp(device_path) })
    else {
        return false;
    };

    bus.wanted_usb_io_dp_list.iter().any(|wanted_path| {
        let Some(header) = device_path_node_from_bytes(wanted_path) else {
            return false;
        };
        if header.r#type != TYPE_MESSAGING {
            return false;
        }

        match header.sub_type {
            device_path_temp::MSG_USB_DP => wanted_path == interface_path,
            device_path_temp::MSG_USB_CLASS_DP => {
                if node_length(&header) != mem::size_of::<UsbClassDevicePath>()
                    || wanted_path.len() < mem::size_of::<UsbClassDevicePath>()
                {
                    return false;
                }
                // SAFETY: The node length was validated, and the packed type has byte alignment.
                let class_path = unsafe { &*wanted_path.as_ptr().cast::<UsbClassDevicePath>() };
                unsafe { match_usb_class(class_path, interface) }
            }
            device_path_temp::MSG_USB_WWID_DP => {
                let node_size = node_length(&header);
                if node_size < mem::size_of::<UsbWwidDevicePath>() || node_size > wanted_path.len() {
                    return false;
                }
                // SAFETY: The complete variable-length WWID node is retained in `wanted_path`.
                let wwid_path = unsafe { &*wanted_path.as_ptr().cast::<UsbWwidDevicePath>() };
                unsafe { match_usb_wwid(wwid_path, interface) }
            }
            _ => false,
        }
    })
}

/// Recursively connects every wanted USB child controller on this bus.
pub fn usb_bus_recursively_connect_wanted_usb_io(
    protocols: &dyn ProtocolServices,
    drivers: &dyn DriverServices,
    usb_bus_id: &EfiUsbBusProtocol,
) -> core::result::Result<(), ProtocolError> {
    // SAFETY: `usb_bus_from_this` validates the containing structure's signature.
    let Some(bus) = (unsafe { usb_bus_from_this(usb_bus_id) }) else {
        return Err(ProtocolError::InvalidParameter);
    };

    let handles = match protocols.locate_handles_for::<UsbIoProtocol>() {
        Ok(handles) => handles,
        Err(ProtocolError::NotFound) => return Ok(()),
        Err(error) => return Err(error),
    };

    // SAFETY: The bus owns a complete device path for its lifetime.
    let Some(bus_path) = (unsafe { bus.device_path.cast::<EfiDevicePathProtocol>().as_ref() }) else {
        return Err(ProtocolError::InvalidParameter);
    };
    let Some(bus_path_size) = (unsafe { device_path_temp_size(bus_path) }) else {
        return Err(ProtocolError::InvalidParameter);
    };
    let bus_path_prefix_size = bus_path_size - mem::size_of::<EfiDevicePathProtocol>();
    let bus_path_ptr = ptr::from_ref(bus_path);

    for handle in handles {
        let is_child = protocols
            .with_protocol_on::<EfiDevicePathProtocol, _>(handle, |device_path| {
                let Some(device_path_size) = (unsafe { device_path_temp_size(device_path) }) else {
                    return false;
                };
                if device_path_size < bus_path_prefix_size {
                    return false;
                }
                let device_path_ptr = ptr::from_ref(device_path);

                // SAFETY: Both paths were validated through their end nodes and contain the compared prefix.
                unsafe {
                    core::slice::from_raw_parts(device_path_ptr.cast::<u8>(), bus_path_prefix_size)
                        == core::slice::from_raw_parts(bus_path_ptr.cast::<u8>(), bus_path_prefix_size)
                }
            })
            .unwrap_or(false);
        if !is_child {
            continue;
        }

        let _ = protocols.with_protocol_on::<UsbIoProtocol, _>(handle, |usb_io| {
            // SAFETY: The bus installs this protocol as the `usb_io` field of `UsbInterface`.
            let Some(interface) = (unsafe { usb_interface_from_usb_io(&usb_io.0) }) else { return };

            if usb_bus_is_wanted_usb_io(bus, interface) && !interface.is_managed.get() {
                let connected = drivers.connect_controller(handle, true).is_ok();
                interface.is_managed.set(connected);
            }
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{is_all_usb_class_device_path, wwid_serial_length};

    #[test]
    fn recognizes_all_usb_class_device_path() {
        let path = [
            0x03, 0x0f, 0x0b, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f, 0xff, 0x04, 0x00,
        ];

        assert!(is_all_usb_class_device_path(&path));
    }

    #[test]
    fn rejects_noncanonical_all_usb_class_device_path() {
        let path = [
            0x03, 0x0f, 0x0b, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f, 0xff, 0x05, 0x00,
        ];

        assert!(!is_all_usb_class_device_path(&path));
    }

    #[test]
    fn validates_wwid_serial_length() {
        assert_eq!(wwid_serial_length(12), Some(1));
        assert_eq!(wwid_serial_length(10), None);
        assert_eq!(wwid_serial_length(11), None);
        assert_eq!(wwid_serial_length(9), None);
    }
}
