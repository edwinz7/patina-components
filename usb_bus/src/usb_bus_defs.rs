//! Rust definitions corresponding to `MdeModulePkg/Bus/Usb/UsbBusDxe/UsbBus.h`.
//!
//! The executable bus implementation is still being migrated. These types keep
//! the private bus model and its timing constants explicit while preserving the
//! pointer-oriented relationships used by the UEFI driver.

#![allow(dead_code)]

#[path = "../../protocols/usb_2_host_controller.rs"]
mod usb_2_host_controller;

use core::ffi::c_void;
use patina::{BinaryGuid, pi::list_entry, protocol::ProtocolInterface, uefi::boot_services::tpl::Tpl};
use r_efi::{
    base::Boolean,
    efi,
    efi::protocols::{device_path::Protocol as DevicePathProtocol, usb_io::Protocol as UsbIoProtocol},
};
use usb_2_host_controller::Protocol as Usb2HcProtocol;

use crate::usb_desc::{UsbConfigDesc, UsbDeviceDesc, UsbEndpointDesc, UsbInterfaceDesc, UsbInterfaceSetting};
use crate::usb_enumer::{UsbHubInit, UsbHubGetPortStatus, UsbHubClearPortChange, UsbHubSetPortFeature, UsbHubClearPortFeature, UsbHubResetPort, UsbHubRelease};

pub const USB_BUS_PROTOCOL_GUID: BinaryGuid = BinaryGuid::from_string("dceefc3d-ad07-4986-be64-f5ba2ed6591c");

pub const USB_MAX_LANG_ID: usize = 16;
pub const USB_MAX_INTERFACE: usize = 16;
pub const USB_MAX_DEVICES: usize = 128;

pub const USB_BUS_1_MILLISECOND: u64 = 1_000;

//
// Roothub and hub's polling interval, set by experience,
// The unit of roothub is 100us, means 100ms as interval, and
// the unit of hub is 1ms, means 64ms as interval.
//
pub const USB_ROOTHUB_POLL_INTERVAL: u64 = 100 * 10_000;
pub const USB_HUB_POLL_INTERVAL: u64 = 64;

//
// defines the minimum number of enumeration attempts
//
pub const USB_ENUM_POLL_MINIMUM_ATTEMPTS: u8 = 6; // MU_CHANGE

//
// defines the maximum number of enumeration attempts
//
pub const USB_ENUM_POLL_MAXIMUM_ATTEMPTS: u8 = 200; // MU_CHANGE

//
// Wait for port stable to work, refers to specification
// [USB20-9.1.2]
//
pub const USB_WAIT_PORT_STABLE_STALL: u64 = 100 * USB_BUS_1_MILLISECOND;

//
// Wait for port statue reg change, set by experience
//
pub const USB_WAIT_PORT_STS_CHANGE_STALL: u64 = 100;

//
// Wait for set device address, refers to specification
// [USB20-9.2.6.3, it says 2ms]
//
pub const USB_SET_DEVICE_ADDRESS_STALL: u64 = 2 * USB_BUS_1_MILLISECOND;

//
// Wait for retry max packet size, set by experience
//
pub const USB_RETRY_MAX_PACK_SIZE_STALL: u64 = 100 * USB_BUS_1_MILLISECOND;

//
// Wait for hub port power-on, refers to specification
// [USB20-11.23.2]
//
pub const USB_SET_PORT_POWER_STALL: u64 = 2 * USB_BUS_1_MILLISECOND;

//
// Wait for port reset, refers to specification
// [USB20-7.1.7.5, it says 10ms for hub and 50ms for
// root hub]
//
// According to USB2.0, Chapter 11.5.1.5 Resetting,
// the worst case for TDRST is 20ms
//
pub const USB_SET_PORT_RESET_STALL: u64 = 20 * USB_BUS_1_MILLISECOND;
pub const USB_SET_ROOT_PORT_RESET_STALL: u64 = 50 * USB_BUS_1_MILLISECOND;

//
// Wait for port recovery to accept SetAddress, refers to specification
// [USB20-7.1.7.5, it says 10 ms for TRSTRCY]
//
pub const USB_SET_PORT_RECOVERY_STALL: u64 = 10 * USB_BUS_1_MILLISECOND;

//
// Wait for clear roothub port reset, set by experience
//
pub const USB_CLR_ROOT_PORT_RESET_STALL: u64 = 20 * USB_BUS_1_MILLISECOND;

//
// Wait for set roothub port enable, set by experience
//
pub const USB_SET_ROOT_PORT_ENABLE_STALL: u64 = 20 * USB_BUS_1_MILLISECOND;

//
// Send general device request timeout.
//
// The USB Specification 2.0, section 11.24.1 recommends a value of
// 50 milliseconds.  We use a value of 500 milliseconds to work
// around slower hubs and devices.
//
pub const USB_GENERAL_DEVICE_REQUEST_TIMEOUT: u32 = 500;

//
// Send clear feature request timeout, set by experience
//
pub const USB_CLEAR_FEATURE_REQUEST_TIMEOUT: u32 = 10;

//
// Bus raises TPL to TPL_NOTIFY to serialize all its operations
// to protect shared data structures.
//
pub const USB_BUS_TPL: Tpl = Tpl::NOTIFY;

pub const USB_INTERFACE_SIGNATURE: u64 = signature(b"USBI");
pub const USB_BUS_SIGNATURE: u64 = signature(b"USBB");

const fn signature(bytes: &[u8; 4]) -> u64 {
    u32::from_le_bytes(*bytes) as u64
}

pub const fn usb_bit(bit: u32) -> usize {
    1usize << bit
}

pub const fn usb_bit_is_set(data: usize, bit: usize) -> bool {
    data & bit == bit
}

//
// Used to locate USB_BUS
// UsbBusProtocol is the private protocol.
// gEfiCallerIdGuid will be used as its protocol guid.
//
#[repr(C)]
pub struct EfiUsbBusProtocol {
    pub reserved: u64,
}

// SAFETY: EfiUsbBusProtocol is the private USB bus protocol installed by this driver.
unsafe impl ProtocolInterface for EfiUsbBusProtocol {
    const PROTOCOL_GUID: BinaryGuid = USB_BUS_PROTOCOL_GUID;
}

//
// Stands for the real USB device. Each device may
// have several separately working interfaces.
//
#[repr(C)]
pub struct UsbDevice {
    pub bus: *mut UsbBus,

    // Configuration information
    pub speed: u8,
    pub address: u8,
    pub max_packet0: u32,

    // The device's descriptors and its configuration
    pub dev_desc: *mut UsbDeviceDesc,
    pub active_config: *mut UsbConfigDesc,

    pub lang_id: [u16; USB_MAX_LANG_ID],
    pub total_lang_id: u16,

    pub num_of_interface: u8,
    pub interfaces: [*mut UsbInterface; USB_MAX_INTERFACE],

    // Parent child relationship
    pub translator: *mut c_void,

    pub parent_addr: u8,
    pub parent_if: *mut UsbInterface,
    pub parent_port: u8,
    pub tier: u8,
    pub connected: Boolean,
    pub disconnect_fail: Boolean,
}

//
// Stands for different functions of USB device
//
#[repr(C)]
pub struct UsbInterface {
    pub signature: usize,
    pub device: *mut UsbDevice,
    pub if_desc: *mut UsbInterfaceDesc,
    pub if_setting: *mut UsbInterfaceSetting,

    // Handles and protocols
    pub handle: efi::Handle,
    pub usb_io: UsbIoProtocol,
    pub device_path: *mut DevicePathProtocol,
    pub is_managed: Boolean,

    // Hub device special data
    pub is_hub: Boolean,
    pub hub_api: *mut UsbHubApi,
    pub num_of_port: u8,
    pub hub_notify: efi::Event,

    // Data used only by normal hub devices
    pub hub_ep: *mut UsbEndpointDesc,
    pub change_map: *mut u8,

    // Data used only by root hub to hand over device to
    // companion UHCI driver if low/full speed devices are
    // connected to EHCI.
    pub max_speed: u8,

    // MU_CHANGE [BEGIN] - 168923
    // Track the the number of enumeration attempts
    pub poll_count: u8,
    // MU_CHANGE [END] - 168923
}

//
// Stands for the current USB Bus
//
#[repr(C)]
pub struct UsbBus {
    pub signature: usize,
    pub bus_id: EfiUsbBusProtocol,

    // Managed USB host controller
    pub host_handle: efi::Handle,
    pub device_path: *mut DevicePathProtocol,
    pub usb2_hc: *mut Usb2HcProtocol,

    // Recorded the max supported usb devices.
    // XHCI can support up to 255 devices.
    // EHCI/UHCI/OHCI supports up to 127 devices.
    pub max_devices: u32,

    // An array of device that is on the bus. Devices[0] is
    // for root hub. Device with address i is at Devices[i].
    pub devices: [*mut UsbDevice; 256],

    // USB Bus driver need to control the recursive connect policy of the bus, only those wanted
    // usb child device will be recursively connected.
    //
    // WantedUsbIoDPList tracks the Usb child devices which user want to recursively fully connecte,
    // every wanted child device is stored in a item of the WantedUsbIoDPList, whose structure is
    // DEVICE_PATH_LIST_ITEM
    pub wanted_usb_io_dp_list: list_entry::Entry,
}

//
// USB Hub Api
//
#[repr(C)]
pub struct UsbHubApi {
    pub init: UsbHubInit,
    pub get_port_status: UsbHubGetPortStatus,
    pub clear_port_change: UsbHubClearPortChange,
    pub set_port_feature: UsbHubSetPortFeature,
    pub clear_port_feature: UsbHubClearPortFeature,
    pub reset_port: UsbHubResetPort,
    pub release: UsbHubRelease,
}

pub const USB_US_LAND_ID: u16 = 0x0409;

pub const DEVICE_PATH_LIST_ITEM_SIGNATURE: u64 = signature(b"dpli");

#[repr(C)]
pub struct DevicePathListItem {
    pub signature: usize,
    pub link: *mut c_void,
    pub device_path: *mut DevicePathProtocol,
}

#[repr(C)]
pub struct UsbClassFormatDevicePath {
    pub usb_class: [u8; 32],
    pub end: DevicePathProtocol,
}
