//! USB bus functions for usb_io.
//!
//! ## License
//!
//! Copyright (c) Microsoft Corporation.
//!
//! SPDX-License-Identifier: Apache-2.0
//!

#![allow(dead_code)]

use crate::efi::protocols::usb_io::{AsyncUsbTransferCallback, Protocol};
use core::ffi::c_void;
//use patina::uefi::boot_services::tpl::Tpl;
use r_efi::{base::Boolean, efi, efi::protocols::usb_io};

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
    _this: *mut Protocol,
    _request: *mut usb_io::DeviceRequest,
    _direction: usb_io::DataDirection,
    _timeout: u32,
    _data: *mut c_void,
    _data_length: usize,
    _usb_status: *mut u32,
) -> efi::Status {
    efi::Status::SUCCESS
}

unsafe extern "efiapi" fn usb_io_bulk_transfer(
    _this: *mut Protocol,
    _endpoint: u8,
    _data: *mut c_void,
    _data_length: *mut usize,
    _timeout: usize,
    _usb_status: *mut u32,
) -> efi::Status {
    efi::Status::SUCCESS
}

unsafe extern "efiapi" fn usb_io_async_interrupt_transfer(
    _this: *mut Protocol,
    _endpoint: u8,
    _is_new_transfer: Boolean,
    _polling_interval: usize,
    _data_length: usize,
    _callback: Option<AsyncUsbTransferCallback>,
    _context: *mut c_void,
) -> efi::Status {
    efi::Status::SUCCESS
}

unsafe extern "efiapi" fn usb_io_sync_interrupt_transfer(
    _this: *mut Protocol,
    _endpoint: u8,
    _data: *mut c_void,
    _data_length: *mut usize,
    _timeout: usize,
    _usb_status: *mut u32,
) -> efi::Status {
    efi::Status::SUCCESS
}

unsafe extern "efiapi" fn usb_io_isochronous_transfer(
    _this: *mut Protocol,
    _endpoint: u8,
    _data: *mut c_void,
    _data_length: usize,
    _usb_status: *mut u32,
) -> efi::Status {
    efi::Status::UNSUPPORTED
}

unsafe extern "efiapi" fn usb_io_async_isochronous_transfer(
    _this: *mut Protocol,
    _endpoint: u8,
    _data: *mut c_void,
    _data_length: usize,
    _callback: AsyncUsbTransferCallback,
    _context: *mut c_void,
) -> efi::Status {
    efi::Status::UNSUPPORTED
}

unsafe extern "efiapi" fn usb_io_get_device_descriptor(
    _this: *mut Protocol,
    _descriptor: *mut usb_io::DeviceDescriptor,
) -> efi::Status {
    efi::Status::SUCCESS
}

unsafe extern "efiapi" fn usb_io_get_config_descriptor(
    _this: *mut Protocol,
    _descriptor: *mut usb_io::ConfigDescriptor,
) -> efi::Status {
    efi::Status::SUCCESS
}

unsafe extern "efiapi" fn usb_io_get_interface_descriptor(
    _this: *mut Protocol,
    _descriptor: *mut usb_io::InterfaceDescriptor,
) -> efi::Status {
    efi::Status::SUCCESS
}

unsafe extern "efiapi" fn usb_io_get_endpoint_descriptor(
    _this: *mut Protocol,
    _index: u8,
    _descriptor: *mut usb_io::EndpointDescriptor,
) -> efi::Status {
    efi::Status::SUCCESS
}

unsafe extern "efiapi" fn usb_io_get_string_descriptor(
    _this: *mut Protocol,
    _language_id: u16,
    _string_index: u8,
    _string: *mut *mut u16,
) -> efi::Status {
    efi::Status::SUCCESS
}

unsafe extern "efiapi" fn usb_io_get_supported_languages(
    _this: *mut Protocol,
    _language_id_table: *mut *mut u16,
    _table_size: *mut u16,
) -> efi::Status {
    efi::Status::SUCCESS
}

unsafe extern "efiapi" fn usb_io_port_reset(_this: *mut usb_io::Protocol) -> efi::Status {
    efi::Status::SUCCESS
}
