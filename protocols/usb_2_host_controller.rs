#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(dead_code)]

//! Rust ABI translation of UEFI `Usb2HostController.h`.
//!
//! The declarations in this module describe the EFI USB 2.0 host controller
//! protocol. Function pointers use the UEFI calling convention and raw
//! pointers preserve the nullable C interface.

use patina::BinaryGuid;
use patina::protocol::ProtocolInterface;
use r_efi::base;

// Placeholder to get things building. Need to reconcile this with the
// below implementation at some point
unsafe impl ProtocolInterface for Protocol {
    const PROTOCOL_GUID: BinaryGuid =
        BinaryGuid::from_fields(0x3e745226, 0x9818, 0x45b6, 0xa2, 0xac, &[0xd7, 0xcd, 0x0e, 0x8b, 0xa2, 0xbc]);
}

pub const PROTOCOL_GUID: base::Guid =
    base::Guid::from_fields(0x3e745226, 0x9818, 0x45b6, 0xa2, 0xac, &[0xd7, 0xcd, 0x0e, 0x8b, 0xa2, 0xbc]);

pub type DataDirection = u32;

#[repr(C, packed)]
pub struct UsbPortStatus {
    pub port_status: u16,
    pub port_change_status: u16,
}

pub const USB_PORT_STAT_CONNECTION: u16 = 0x0001;
pub const USB_PORT_STAT_ENABLE: u16 = 0x0002;
pub const USB_PORT_STAT_SUSPEND: u16 = 0x0004;
pub const USB_PORT_STAT_OVERCURRENT: u16 = 0x0008;
pub const USB_PORT_STAT_RESET: u16 = 0x0010;
pub const USB_PORT_STAT_POWER: u16 = 0x0100;
pub const USB_PORT_STAT_LOW_SPEED: u16 = 0x0200;
pub const USB_PORT_STAT_HIGH_SPEED: u16 = 0x0400;
pub const USB_PORT_STAT_SUPER_SPEED: u16 = 0x0800;
pub const USB_PORT_STAT_OWNER: u16 = 0x2000;

pub const USB_PORT_STAT_C_CONNECTION: u16 = 0x0001;
pub const USB_PORT_STAT_C_ENABLE: u16 = 0x0002;
pub const USB_PORT_STAT_C_SUSPEND: u16 = 0x0004;
pub const USB_PORT_STAT_C_OVERCURRENT: u16 = 0x0008;
pub const USB_PORT_STAT_C_RESET: u16 = 0x0010;

#[repr(u16)]
pub enum UsbPortFeature {
    Enable = 1,
    Suspend = 2,
    Reset = 4,
    Power = 8,
    Owner = 13,
    ConnectChange = 16,
    EnableChange = 17,
    SuspendChange = 18,
    OverCurrentChange = 19,
    ResetChange = 20,
}

pub const EFI_USB_SPEED_FULL: u8 = 0x00;
pub const EFI_USB_SPEED_LOW: u8 = 0x01;
pub const EFI_USB_SPEED_HIGH: u8 = 0x02;
pub const EFI_USB_SPEED_SUPER: u8 = 0x03;

#[repr(C, packed)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Usb2HcTransactionTranslator {
    pub translator_hub_address: u8,
    pub translator_port_number: u8,
}

#[repr(u32)]
pub enum UsbHcState {
    Halt = 0,
    Operational = 1,
    Suspend = 2,
    Maximum = 3,
}

pub const EFI_USB_HC_RESET_GLOBAL: u16 = 0x0001;
pub const EFI_USB_HC_RESET_HOST_CONTROLLER: u16 = 0x0002;
pub const EFI_USB_HC_RESET_GLOBAL_WITH_DEBUG: u16 = 0x0004;
pub const EFI_USB_HC_RESET_HOST_WITH_DEBUG: u16 = 0x0008;
pub const EFI_USB_MAX_BULK_BUFFER_NUM: usize = 10;
pub const EFI_USB_MAX_ISO_BUFFER_NUM: usize = 7;
pub const EFI_USB_MAX_ISO_BUFFER_NUM1: usize = 2;

// These types are declared by Protocol/UsbIo.h in the EDK II headers.
#[repr(C)]
pub struct UsbDeviceRequest {
    pub request_type: u8,
    pub request: u8,
    pub value: u16,
    pub index: u16,
    pub length: u16,
}

#[repr(u32)]
pub enum UsbDataDirection {
    In = 0,
    Out = 1,
    NoData = 2,
}

pub type AsyncUsbTransferCallback =
    unsafe extern "efiapi" fn(*mut core::ffi::c_void, usize, *mut core::ffi::c_void, u32) -> base::Status;

pub type ProtocolGetCapability = unsafe extern "efiapi" fn(*mut Protocol, *mut u8, *mut u8, *mut u8) -> base::Status;

pub type ProtocolReset = unsafe extern "efiapi" fn(*mut Protocol, u16) -> base::Status;

pub type ProtocolGetState = unsafe extern "efiapi" fn(*mut Protocol, *mut UsbHcState) -> base::Status;

pub type ProtocolSetState = unsafe extern "efiapi" fn(*mut Protocol, UsbHcState) -> base::Status;

pub type ProtocolControlTransfer = unsafe extern "efiapi" fn(
    *mut Protocol,
    u8,
    u8,
    usize,
    *mut UsbDeviceRequest,
    UsbDataDirection,
    *mut core::ffi::c_void,
    *mut usize,
    usize,
    *mut Usb2HcTransactionTranslator,
    *mut u32,
) -> base::Status;

pub type ProtocolBulkTransfer = unsafe extern "efiapi" fn(
    *mut Protocol,
    u8,
    u8,
    u8,
    usize,
    u8,
    *mut *mut core::ffi::c_void,
    *mut usize,
    *mut u8,
    usize,
    *mut Usb2HcTransactionTranslator,
    *mut u32,
) -> base::Status;

pub type ProtocolAsyncInterruptTransfer = unsafe extern "efiapi" fn(
    *mut Protocol,
    u8,
    u8,
    u8,
    usize,
    base::Boolean,
    *mut u8,
    usize,
    usize,
    *mut Usb2HcTransactionTranslator,
    Option<AsyncUsbTransferCallback>,
    *mut core::ffi::c_void,
) -> base::Status;

pub type ProtocolSyncInterruptTransfer = unsafe extern "efiapi" fn(
    *mut Protocol,
    u8,
    u8,
    u8,
    usize,
    *mut core::ffi::c_void,
    *mut usize,
    *mut u8,
    usize,
    *mut Usb2HcTransactionTranslator,
    *mut u32,
) -> base::Status;

pub type ProtocolIsochronousTransfer = unsafe extern "efiapi" fn(
    *mut Protocol,
    u8,
    u8,
    u8,
    usize,
    u8,
    *mut *mut core::ffi::c_void,
    usize,
    *mut Usb2HcTransactionTranslator,
    *mut u32,
) -> base::Status;

pub type ProtocolAsyncIsochronousTransfer = unsafe extern "efiapi" fn(
    *mut Protocol,
    u8,
    u8,
    u8,
    usize,
    u8,
    *mut *mut core::ffi::c_void,
    usize,
    *mut Usb2HcTransactionTranslator,
    Option<AsyncUsbTransferCallback>,
    *mut core::ffi::c_void,
) -> base::Status;

pub type ProtocolGetRootHubPortStatus =
    unsafe extern "efiapi" fn(*mut Protocol, u8, *mut UsbPortStatus) -> base::Status;

pub type ProtocolSetRootHubPortFeature = unsafe extern "efiapi" fn(*mut Protocol, u8, UsbPortFeature) -> base::Status;

pub type ProtocolClearRootHubPortFeature = unsafe extern "efiapi" fn(*mut Protocol, u8, UsbPortFeature) -> base::Status;

#[repr(C)]
pub struct Protocol {
    pub get_capability: ProtocolGetCapability,
    pub reset: ProtocolReset,
    pub get_state: ProtocolGetState,
    pub set_state: ProtocolSetState,
    pub control_transfer: ProtocolControlTransfer,
    pub bulk_transfer: ProtocolBulkTransfer,
    pub async_interrupt_transfer: ProtocolAsyncInterruptTransfer,
    pub sync_interrupt_transfer: ProtocolSyncInterruptTransfer,
    pub isochronous_transfer: ProtocolIsochronousTransfer,
    pub async_isochronous_transfer: ProtocolAsyncIsochronousTransfer,
    pub get_root_hub_port_status: ProtocolGetRootHubPortStatus,
    pub set_root_hub_port_feature: ProtocolSetRootHubPortFeature,
    pub clear_root_hub_port_feature: ProtocolClearRootHubPortFeature,
    pub major_revision: u16,
    pub minor_revision: u16,
}
