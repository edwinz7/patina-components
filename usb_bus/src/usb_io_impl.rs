//! USB bus functions for usb_io.
//!
//! ## License
//!
//! Copyright (c) Microsoft Corporation.
//!
//! SPDX-License-Identifier: Apache-2.0
//!

#![allow(dead_code)]

use crate::{
    efi::protocols::usb_io::{AsyncUsbTransferCallback, Protocol},
    usb_2_host_controller::{UsbDataDirection, UsbDeviceRequest},
    usb_bus_defs::{USB_BUS_TPL, USB_INTERFACE_SIGNATURE, USB_SET_DEVICE_ADDRESS_STALL, UsbInterface},
    usb_desc::{
        usb_build_desc_table, usb_free_dev_desc, usb_get_max_packet_size0, usb_get_one_string, usb_set_address,
        usb_set_config, usb_update_descriptors,
    },
    usb_enumer::{usb_get_endpoint_desc, usb_remove_config, usb_select_config, usb_select_setting},
    usb_hub::{usb_endpoint_addr, usb_endpoint_type, usb_hub_ctrl_clear_tt_buffer},
    usb_utility::{
        usb_hc_async_interrupt_transfer, usb_hc_bulk_transfer, usb_hc_control_transfer, usb_hc_sync_interrupt_transfer,
    },
};
use alloc::alloc::{Layout, alloc_zeroed};
use alloc::boxed::Box;
use core::{ffi::c_void, mem::offset_of, ptr, time::Duration};
use patina::component::service::uefi_services::tpl::TplServicesExt;
use r_efi::{base::Boolean, efi::Status, efi::protocols::usb_io};

const USB_REQ_CLEAR_FEATURE: u8 = 0x01;
const USB_REQ_SET_CONFIG: u8 = 0x09;
const USB_REQ_SET_INTERFACE: u8 = 0x0b;
const USB_REQUEST_TYPE_STANDARD_DEVICE: u8 = 0x00;
const USB_REQUEST_TYPE_STANDARD_INTERFACE: u8 = 0x01;
const USB_REQUEST_TYPE_STANDARD_ENDPOINT: u8 = 0x02;
const USB_FEATURE_ENDPOINT_HALT: u16 = 0;
const USB_ENDPOINT_CONTROL: u16 = 0;
const USB_ENDPOINT_BULK: u16 = 2;
const USB_ENDPOINT_INTERRUPT: u16 = 3;

/// Creates a new `UsbIoProtocol` populated with this module's function pointers.
pub fn new_usb_io_protocol() -> Protocol {
    Protocol {
        control_transfer: usb_io_control_transfer,
        bulk_transfer: usb_io_bulk_transfer,
        async_interrupt_transfer: usb_io_async_interrupt_transfer,
        sync_interrupt_transfer: usb_io_sync_interrupt_transfer,
        isochronous_transfer: usb_io_isochronous_transfer,
        async_isochronous_transfer: usb_io_async_isochronous_transfer,
        get_device_descriptor: usb_io_get_device_descriptor,
        get_config_descriptor: usb_io_get_config_descriptor,
        get_interface_descriptor: usb_io_get_interface_descriptor,
        get_endpoint_descriptor: usb_io_get_endpoint_descriptor,
        get_string_descriptor: usb_io_get_string_descriptor,
        get_supported_languages: usb_io_get_supported_languages,
        port_reset: usb_io_port_reset,
    }
}

unsafe extern "efiapi" fn usb_io_control_transfer(
    this: *mut Protocol,
    request: *mut usb_io::DeviceRequest,
    direction: usb_io::DataDirection,
    timeout: u32,
    data: *mut c_void,
    data_length: usize,
    usb_status: *mut u32,
) -> Status {
    if this.is_null() || request.is_null() || usb_status.is_null() {
        return Status::INVALID_PARAMETER;
    }

    let interface = unsafe { this.byte_sub(offset_of!(UsbInterface, usb_io)).cast::<UsbInterface>() };
    let device = unsafe { (*interface).device };
    if device.is_null() {
        return Status::INVALID_PARAMETER;
    }
    let bus = unsafe { (*device).bus };
    if bus.is_null() {
        return Status::INVALID_PARAMETER;
    }
    let tpl = unsafe { (*bus).hub_services.tpl };
    let _tpl_guard = tpl.raise(USB_BUS_TPL);

    if unsafe { (*interface).signature } != USB_INTERFACE_SIGNATURE as usize
        || unsafe { (*interface).device } != device
        || unsafe { (*device).bus } != bus
    {
        return Status::INVALID_PARAMETER;
    }

    if !unsafe { (*device).connected } {
        return Status::DEVICE_ERROR;
    }

    let host_direction = match direction {
        usb_io::DATA_IN => UsbDataDirection::In,
        usb_io::DATA_OUT => UsbDataDirection::Out,
        usb_io::NO_DATA => UsbDataDirection::NoData,
        _ => return Status::INVALID_PARAMETER,
    };
    let requested_data_length = data_length;
    let mut actual_data_length = data_length;
    let mut status = unsafe {
        usb_hc_control_transfer(
            bus,
            (*device).address,
            (*device).speed,
            (*device).max_packet0 as usize,
            request.cast::<UsbDeviceRequest>(),
            host_direction,
            data,
            &mut actual_data_length,
            timeout as usize,
            ptr::addr_of_mut!((*device).translator),
            usb_status,
        )
    };

    if !status.is_error() && direction != usb_io::NO_DATA && actual_data_length != requested_data_length {
        return Status::DEVICE_ERROR;
    }

    if status.is_error() || unsafe { *usb_status } != usb_io::NOERROR {
        let translator_address = unsafe { (*device).translator.translator_hub_address as usize };
        let max_devices = unsafe { ((*bus).max_devices as usize).min((*bus).devices.len()) };
        if translator_address != 0 && translator_address < max_devices {
            if let Some(translator) = unsafe { (*bus).devices[translator_address].as_mut() } {
                let _ = unsafe {
                    usb_hub_ctrl_clear_tt_buffer(
                        translator,
                        (*device).translator.translator_port_number,
                        (*device).address as u16,
                        0,
                        USB_ENDPOINT_CONTROL,
                    )
                };
            }
        }
        return status;
    }

    let request_ref = unsafe { &*request };
    if request_ref.request == USB_REQ_CLEAR_FEATURE
        && request_ref.request_type == USB_REQUEST_TYPE_STANDARD_ENDPOINT
        && request_ref.value == USB_FEATURE_ENDPOINT_HALT
    {
        if let Some(endpoint) = usb_get_endpoint_desc(unsafe { &mut *interface }, request_ref.index as u8) {
            endpoint.toggle = 0;
        }
    }

    if request_ref.request == USB_REQ_SET_CONFIG && request_ref.request_type == USB_REQUEST_TYPE_STANDARD_DEVICE {
        if let Some(active_config) = unsafe { (*device).active_config.as_ref() }
            && request_ref.value == active_config.descriptor.configuration_value as u16
        {
            return status;
        }

        if !unsafe { (*device).active_config.is_null() } {
            let _ = unsafe { usb_remove_config(&mut *device) };
        }
        if request_ref.value != 0 {
            status = unsafe { usb_select_config(&mut *device, request_ref.value as u8) };
        }
        return status;
    }

    if request_ref.request == USB_REQ_SET_INTERFACE
        && request_ref.request_type == USB_REQUEST_TYPE_STANDARD_INTERFACE
        && request_ref.index == unsafe { (*(*interface).if_setting).descriptor.interface_number as u16 }
    {
        status = unsafe { usb_select_setting(&mut *(*interface).if_desc, request_ref.value as u8) };
        if !status.is_error() {
            unsafe {
                (*interface).if_setting = (*(*interface).if_desc).settings[(*(*interface).if_desc).active_index];
            }
        }
    }

    status
}

unsafe extern "efiapi" fn usb_io_bulk_transfer(
    this: *mut Protocol,
    endpoint: u8,
    data: *mut c_void,
    data_length: *mut usize,
    timeout: usize,
    usb_status: *mut u32,
) -> Status {
    let endpoint_number = usb_endpoint_addr(endpoint);
    if this.is_null() || endpoint_number == 0 || endpoint_number > 15 || usb_status.is_null() {
        return Status::INVALID_PARAMETER;
    }

    let interface = unsafe { this.byte_sub(offset_of!(UsbInterface, usb_io)).cast::<UsbInterface>() };
    let device = unsafe { (*interface).device };
    if device.is_null() {
        return Status::INVALID_PARAMETER;
    }
    let bus = unsafe { (*device).bus };
    if bus.is_null() {
        return Status::INVALID_PARAMETER;
    }
    let tpl = unsafe { (*bus).hub_services.tpl };
    let _tpl_guard = tpl.raise(USB_BUS_TPL);

    if unsafe { (*interface).signature } != USB_INTERFACE_SIGNATURE as usize
        || unsafe { (*interface).device } != device
        || unsafe { (*device).bus } != bus
    {
        return Status::INVALID_PARAMETER;
    }

    if !unsafe { (*device).connected } {
        return Status::DEVICE_ERROR;
    }

    let Some(endpoint_desc) = usb_get_endpoint_desc(unsafe { &mut *interface }, endpoint) else {
        return Status::INVALID_PARAMETER;
    };
    if usb_endpoint_type(endpoint_desc.descriptor.attributes) != USB_ENDPOINT_BULK as u8 {
        return Status::INVALID_PARAMETER;
    }

    let mut data_buffer = data;
    let mut toggle = endpoint_desc.toggle;
    let status = unsafe {
        usb_hc_bulk_transfer(
            bus,
            (*device).address,
            endpoint,
            (*device).speed,
            endpoint_desc.descriptor.max_packet_size as usize,
            1,
            ptr::from_mut(&mut data_buffer),
            data_length,
            ptr::from_mut(&mut toggle),
            timeout,
            ptr::addr_of_mut!((*device).translator),
            usb_status,
        )
    };

    endpoint_desc.toggle = toggle;

    if status.is_error() || unsafe { *usb_status } != usb_io::NOERROR {
        let translator_address = unsafe { (*device).translator.translator_hub_address as usize };
        let max_devices = unsafe { ((*bus).max_devices as usize).min((*bus).devices.len()) };
        if translator_address != 0 && translator_address < max_devices {
            if let Some(translator) = unsafe { (*bus).devices[translator_address].as_mut() } {
                let _ = unsafe {
                    usb_hub_ctrl_clear_tt_buffer(
                        translator,
                        (*device).translator.translator_port_number,
                        (*device).address as u16,
                        0,
                        USB_ENDPOINT_BULK,
                    )
                };
            }
        }
    }

    status
}

unsafe extern "efiapi" fn usb_io_sync_interrupt_transfer(
    this: *mut Protocol,
    endpoint: u8,
    data: *mut c_void,
    data_length: *mut usize,
    timeout: usize,
    usb_status: *mut u32,
) -> Status {
    let endpoint_number = usb_endpoint_addr(endpoint);
    if this.is_null() || endpoint_number == 0 || endpoint_number > 15 || usb_status.is_null() {
        return Status::INVALID_PARAMETER;
    }

    let interface = unsafe { this.byte_sub(offset_of!(UsbInterface, usb_io)).cast::<UsbInterface>() };
    let device = unsafe { (*interface).device };
    if device.is_null() {
        return Status::INVALID_PARAMETER;
    }
    let bus = unsafe { (*device).bus };
    if bus.is_null() {
        return Status::INVALID_PARAMETER;
    }
    let tpl = unsafe { (*bus).hub_services.tpl };
    let _tpl_guard = tpl.raise(USB_BUS_TPL);

    if unsafe { (*interface).signature } != USB_INTERFACE_SIGNATURE as usize
        || unsafe { (*interface).device } != device
        || unsafe { (*device).bus } != bus
    {
        return Status::INVALID_PARAMETER;
    }

    if !unsafe { (*device).connected } {
        return Status::DEVICE_ERROR;
    }

    let Some(endpoint_desc) = usb_get_endpoint_desc(unsafe { &mut *interface }, endpoint) else {
        return Status::INVALID_PARAMETER;
    };
    if usb_endpoint_type(endpoint_desc.descriptor.attributes) != USB_ENDPOINT_INTERRUPT as u8 {
        return Status::INVALID_PARAMETER;
    }

    let mut toggle = endpoint_desc.toggle;
    let status = unsafe {
        usb_hc_sync_interrupt_transfer(
            bus,
            (*device).address,
            endpoint,
            (*device).speed,
            endpoint_desc.descriptor.max_packet_size as usize,
            data,
            data_length,
            ptr::from_mut(&mut toggle),
            timeout,
            ptr::addr_of_mut!((*device).translator),
            usb_status,
        )
    };

    endpoint_desc.toggle = toggle;
    status
}

unsafe extern "efiapi" fn usb_io_async_interrupt_transfer(
    this: *mut Protocol,
    endpoint: u8,
    is_new_transfer: Boolean,
    polling_interval: usize,
    data_length: usize,
    callback: Option<AsyncUsbTransferCallback>,
    context: *mut c_void,
) -> Status {
    let endpoint_number = usb_endpoint_addr(endpoint);
    if this.is_null() || endpoint_number == 0 || endpoint_number > 15 {
        return Status::INVALID_PARAMETER;
    }

    let interface = unsafe { this.byte_sub(offset_of!(UsbInterface, usb_io)).cast::<UsbInterface>() };
    let device = unsafe { (*interface).device };
    if device.is_null() {
        return Status::INVALID_PARAMETER;
    }
    let bus = unsafe { (*device).bus };
    if bus.is_null() {
        return Status::INVALID_PARAMETER;
    }
    let tpl = unsafe { (*bus).hub_services.tpl };
    let _tpl_guard = tpl.raise(USB_BUS_TPL);

    if unsafe { (*interface).signature } != USB_INTERFACE_SIGNATURE as usize
        || unsafe { (*interface).device } != device
        || unsafe { (*device).bus } != bus
    {
        return Status::INVALID_PARAMETER;
    }

    if !unsafe { (*device).connected } && is_new_transfer != false {
        return Status::DEVICE_ERROR;
    }

    let Some(endpoint_desc) = usb_get_endpoint_desc(unsafe { &mut *interface }, endpoint) else {
        return Status::INVALID_PARAMETER;
    };
    if usb_endpoint_type(endpoint_desc.descriptor.attributes) != USB_ENDPOINT_INTERRUPT as u8 {
        return Status::INVALID_PARAMETER;
    }

    let mut toggle = endpoint_desc.toggle;
    let status = unsafe {
        usb_hc_async_interrupt_transfer(
            bus,
            (*device).address,
            endpoint,
            (*device).speed,
            endpoint_desc.descriptor.max_packet_size as usize,
            is_new_transfer,
            ptr::from_mut(&mut toggle),
            polling_interval,
            data_length,
            ptr::addr_of_mut!((*device).translator),
            callback,
            context,
        )
    };

    endpoint_desc.toggle = toggle;
    status
}

unsafe extern "efiapi" fn usb_io_isochronous_transfer(
    _this: *mut Protocol,
    _endpoint: u8,
    _data: *mut c_void,
    _data_length: usize,
    _usb_status: *mut u32,
) -> Status {
    Status::UNSUPPORTED
}

unsafe extern "efiapi" fn usb_io_async_isochronous_transfer(
    _this: *mut Protocol,
    _endpoint: u8,
    _data: *mut c_void,
    _data_length: usize,
    _callback: AsyncUsbTransferCallback,
    _context: *mut c_void,
) -> Status {
    Status::UNSUPPORTED
}

unsafe extern "efiapi" fn usb_io_get_device_descriptor(
    this: *mut Protocol,
    descriptor: *mut usb_io::DeviceDescriptor,
) -> Status {
    if this.is_null() || descriptor.is_null() {
        return Status::INVALID_PARAMETER;
    }

    let interface = unsafe { this.byte_sub(offset_of!(UsbInterface, usb_io)).cast::<UsbInterface>() };
    let device = unsafe { (*interface).device };
    if device.is_null() {
        return Status::INVALID_PARAMETER;
    }
    let bus = unsafe { (*device).bus };
    if bus.is_null() {
        return Status::INVALID_PARAMETER;
    }
    let tpl = unsafe { (*bus).hub_services.tpl };
    let _tpl_guard = tpl.raise(USB_BUS_TPL);

    if unsafe { (*interface).signature } != USB_INTERFACE_SIGNATURE as usize
        || unsafe { (*interface).device } != device
        || unsafe { (*device).bus } != bus
    {
        return Status::INVALID_PARAMETER;
    }
    if !unsafe { (*device).connected } {
        return Status::DEVICE_ERROR;
    }
    if unsafe { (*device).dev_desc.is_null() } {
        return Status::NOT_FOUND;
    }

    unsafe {
        ptr::copy_nonoverlapping(ptr::addr_of!((*(*device).dev_desc).descriptor), descriptor, 1);
    }
    Status::SUCCESS
}

unsafe extern "efiapi" fn usb_io_get_config_descriptor(
    this: *mut Protocol,
    descriptor: *mut usb_io::ConfigDescriptor,
) -> Status {
    if this.is_null() || descriptor.is_null() {
        return Status::INVALID_PARAMETER;
    }

    let interface = unsafe { this.byte_sub(offset_of!(UsbInterface, usb_io)).cast::<UsbInterface>() };
    let device = unsafe { (*interface).device };
    if device.is_null() {
        return Status::INVALID_PARAMETER;
    }
    let bus = unsafe { (*device).bus };
    if bus.is_null() {
        return Status::INVALID_PARAMETER;
    }
    let tpl = unsafe { (*bus).hub_services.tpl };
    let _tpl_guard = tpl.raise(USB_BUS_TPL);

    if unsafe { (*interface).signature } != USB_INTERFACE_SIGNATURE as usize
        || unsafe { (*interface).device } != device
        || unsafe { (*device).bus } != bus
    {
        return Status::INVALID_PARAMETER;
    }
    if !unsafe { (*device).connected } {
        return Status::DEVICE_ERROR;
    }
    if unsafe { (*device).active_config.is_null() } {
        return Status::NOT_FOUND;
    }

    unsafe {
        ptr::copy_nonoverlapping(ptr::addr_of!((*(*device).active_config).descriptor), descriptor, 1);
    }
    Status::SUCCESS
}

unsafe extern "efiapi" fn usb_io_get_interface_descriptor(
    this: *mut Protocol,
    descriptor: *mut usb_io::InterfaceDescriptor,
) -> Status {
    if this.is_null() || descriptor.is_null() {
        return Status::INVALID_PARAMETER;
    }

    let interface = unsafe { this.byte_sub(offset_of!(UsbInterface, usb_io)).cast::<UsbInterface>() };
    let device = unsafe { (*interface).device };
    if device.is_null() {
        return Status::INVALID_PARAMETER;
    }
    let bus = unsafe { (*device).bus };
    if bus.is_null() {
        return Status::INVALID_PARAMETER;
    }
    let tpl = unsafe { (*bus).hub_services.tpl };
    let _tpl_guard = tpl.raise(USB_BUS_TPL);

    if unsafe { (*interface).signature } != USB_INTERFACE_SIGNATURE as usize
        || unsafe { (*interface).device } != device
        || unsafe { (*device).bus } != bus
        || unsafe { (*interface).if_setting.is_null() }
    {
        return Status::INVALID_PARAMETER;
    }
    if !unsafe { (*device).connected } {
        return Status::DEVICE_ERROR;
    }

    unsafe {
        ptr::copy_nonoverlapping(ptr::addr_of!((*(*interface).if_setting).descriptor), descriptor, 1);
    }
    Status::SUCCESS
}

unsafe extern "efiapi" fn usb_io_get_endpoint_descriptor(
    this: *mut Protocol,
    index: u8,
    descriptor: *mut usb_io::EndpointDescriptor,
) -> Status {
    if this.is_null() {
        return Status::INVALID_PARAMETER;
    }

    let interface = unsafe { this.byte_sub(offset_of!(UsbInterface, usb_io)).cast::<UsbInterface>() };
    let device = unsafe { (*interface).device };
    if device.is_null() {
        return Status::INVALID_PARAMETER;
    }
    let bus = unsafe { (*device).bus };
    if bus.is_null() {
        return Status::INVALID_PARAMETER;
    }
    let tpl = unsafe { (*bus).hub_services.tpl };
    let _tpl_guard = tpl.raise(USB_BUS_TPL);

    if unsafe { (*interface).signature } != USB_INTERFACE_SIGNATURE as usize
        || unsafe { (*interface).device } != device
        || unsafe { (*device).bus } != bus
        || unsafe { (*interface).if_setting.is_null() }
    {
        return Status::INVALID_PARAMETER;
    }
    if !unsafe { (*device).connected } {
        return Status::DEVICE_ERROR;
    }
    if descriptor.is_null() || index > 15 {
        return Status::INVALID_PARAMETER;
    }

    let setting = unsafe { (*interface).if_setting };
    if index >= unsafe { (*setting).descriptor.num_endpoints } {
        return Status::NOT_FOUND;
    }
    let endpoint = unsafe { *(*setting).endpoints.add(index as usize) };
    if endpoint.is_null() {
        return Status::NOT_FOUND;
    }

    unsafe {
        ptr::copy_nonoverlapping(ptr::addr_of!((*endpoint).descriptor), descriptor, 1);
    }
    Status::SUCCESS
}

unsafe extern "efiapi" fn usb_io_get_string_descriptor(
    this: *mut Protocol,
    language_id: u16,
    string_index: u8,
    string: *mut *mut u16,
) -> Status {
    if string_index == 0 || language_id == 0 {
        return Status::NOT_FOUND;
    }
    if this.is_null() || string.is_null() {
        return Status::INVALID_PARAMETER;
    }

    let interface = unsafe { this.byte_sub(offset_of!(UsbInterface, usb_io)).cast::<UsbInterface>() };
    let device = unsafe { (*interface).device };
    if device.is_null() {
        return Status::INVALID_PARAMETER;
    }
    let bus = unsafe { (*device).bus };
    if bus.is_null() {
        return Status::INVALID_PARAMETER;
    }
    let tpl = unsafe { (*bus).hub_services.tpl };
    let _tpl_guard = tpl.raise(USB_BUS_TPL);

    if unsafe { (*interface).signature } != USB_INTERFACE_SIGNATURE as usize
        || unsafe { (*interface).device } != device
        || unsafe { (*device).bus } != bus
    {
        return Status::INVALID_PARAMETER;
    }
    if !unsafe { (*device).connected } {
        return Status::DEVICE_ERROR;
    }

    let languages = unsafe { &(*device).lang_id };
    let language_count = (unsafe { (*device).total_lang_id as usize }).min(languages.len());
    if !languages[..language_count].contains(&language_id) {
        return Status::NOT_FOUND;
    }

    let Some(string_units) = (unsafe { usb_get_one_string(&*device, string_index, language_id) }) else {
        return Status::NOT_FOUND;
    };
    if string_units.is_empty() {
        return Status::NOT_FOUND;
    }

    let Some(output_length) = string_units.len().checked_add(1) else {
        return Status::OUT_OF_RESOURCES;
    };
    let Ok(layout) = Layout::array::<u16>(output_length) else {
        return Status::OUT_OF_RESOURCES;
    };
    let output = unsafe { alloc_zeroed(layout).cast::<u16>() };
    if output.is_null() {
        return Status::OUT_OF_RESOURCES;
    }
    unsafe {
        ptr::copy_nonoverlapping(string_units.as_ptr(), output, string_units.len());
        *string = output;
    }
    Status::SUCCESS
}

unsafe extern "efiapi" fn usb_io_get_supported_languages(
    this: *mut Protocol,
    language_id_table: *mut *mut u16,
    table_size: *mut u16,
) -> Status {
    if this.is_null() || language_id_table.is_null() || table_size.is_null() {
        return Status::INVALID_PARAMETER;
    }

    let interface = unsafe { this.byte_sub(offset_of!(UsbInterface, usb_io)).cast::<UsbInterface>() };
    let device = unsafe { (*interface).device };
    if device.is_null() {
        return Status::INVALID_PARAMETER;
    }
    let bus = unsafe { (*device).bus };
    if bus.is_null() {
        return Status::INVALID_PARAMETER;
    }
    let tpl = unsafe { (*bus).hub_services.tpl };
    let _tpl_guard = tpl.raise(USB_BUS_TPL);

    if unsafe { (*interface).signature } != USB_INTERFACE_SIGNATURE as usize
        || unsafe { (*interface).device } != device
        || unsafe { (*device).bus } != bus
    {
        return Status::INVALID_PARAMETER;
    }
    if !unsafe { (*device).connected } {
        return Status::DEVICE_ERROR;
    }

    unsafe {
        *language_id_table = ptr::addr_of_mut!((*device).lang_id).cast::<u16>();
        *table_size = (*device).total_lang_id.saturating_mul(core::mem::size_of::<u16>() as u16);
    }
    Status::SUCCESS
}

unsafe extern "efiapi" fn usb_io_port_reset(this: *mut usb_io::Protocol) -> Status {
    if this.is_null() {
        return Status::INVALID_PARAMETER;
    }

    let interface = unsafe { this.byte_sub(offset_of!(UsbInterface, usb_io)).cast::<UsbInterface>() };
    let device = unsafe { (*interface).device };
    if device.is_null() {
        return Status::INVALID_PARAMETER;
    }
    let bus = unsafe { (*device).bus };
    if bus.is_null() {
        return Status::INVALID_PARAMETER;
    }
    let services = unsafe { (*bus).hub_services };
    let _tpl_guard = services.tpl.raise(USB_BUS_TPL);

    if unsafe { (*interface).signature } != USB_INTERFACE_SIGNATURE as usize
        || unsafe { (*interface).device } != device
        || unsafe { (*device).bus } != bus
    {
        return Status::INVALID_PARAMETER;
    }
    if !unsafe { (*device).connected } {
        return Status::DEVICE_ERROR;
    }
    if unsafe { (*interface).is_hub } {
        return Status::INVALID_PARAMETER;
    }

    let parent_interface = unsafe { (*device).parent_if };
    let Some(hub_api) = (unsafe { parent_interface.as_ref().and_then(|parent| parent.hub_api) }) else {
        return Status::INVALID_PARAMETER;
    };
    let parent_port = unsafe { (*device).parent_port };
    let status = unsafe { (hub_api.reset_port)(&mut *parent_interface, parent_port, &services) };
    if status.is_error() {
        return status;
    }

    unsafe { (hub_api.clear_port_change)(&mut *parent_interface, parent_port) };

    let device_address = unsafe { (*device).address };
    unsafe { (*device).address = 0 };
    let status = unsafe { usb_set_address(&mut *device, device_address) };
    unsafe { (*device).address = device_address };

    let _ = services.timing.stall(Duration::from_micros(USB_SET_DEVICE_ADDRESS_STALL));
    if status.is_error() {
        return status;
    }

    unsafe { usb_update_descriptors(&mut *device) };

    if unsafe { !(*device).active_config.is_null() } {
        let device_descriptor = unsafe { (*device).dev_desc };
        unsafe { (*device).dev_desc = ptr::null_mut() };
        if !device_descriptor.is_null() {
            unsafe { usb_free_dev_desc(Box::from_raw(device_descriptor)) };
        }

        let _ = unsafe { usb_remove_config(&mut *device) };
        let _ = unsafe { usb_get_max_packet_size0(&mut *device) };
        let status = unsafe { usb_build_desc_table(&mut *device) };
        if status.is_error() {
            return status;
        }

        let device_descriptor = unsafe { (*device).dev_desc };
        if device_descriptor.is_null() || unsafe { (*device_descriptor).configs.is_null() } {
            return Status::DEVICE_ERROR;
        }
        let configuration = unsafe { *(*device_descriptor).configs };
        if configuration.is_null() {
            return Status::DEVICE_ERROR;
        }
        let configuration_value = unsafe { (*configuration).descriptor.configuration_value };

        let _ = unsafe { usb_set_config(&mut *device, configuration_value) };
        return unsafe { usb_select_config(&mut *device, configuration_value) };
    }

    status
}
