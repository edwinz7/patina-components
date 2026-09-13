//! USB hub definitions translated from `UsbHub.h`.

#![allow(dead_code)]

use r_efi::efi;

use crate::usb_2_host_controller::UsbPortFeature;
use crate::usb_bus_defs::{UsbDevice, UsbInterface};

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
