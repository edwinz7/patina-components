//! USB HID Driver — produces the HidIo protocol on USB HID devices.
//!
//! This crate implements a UEFI Driver Binding that consumes the USB IO protocol
//! and produces the HidIo protocol for each USB HID device it manages.
//!
//! ## License
//!
//! Copyright (c) Microsoft Corporation.
//!
//! SPDX-License-Identifier: Apache-2.0
//!
#![cfg_attr(not(test), no_std)]
#![feature(coverage_attribute)]

extern crate alloc;

pub(crate) mod driver;
pub(crate) mod usb_bus_defs;
pub(crate) mod usb_enumer;
pub(crate) mod usb_desc;
pub(crate) mod usb_hub;
pub(crate) mod usb_io_impl;
pub(crate) mod usb_utility;

//#[cfg(test)]
//pub(crate) mod test_stubs;

use alloc::boxed::Box;
use r_efi::efi;

use patina::{
    BinaryGuid,
    component::{component, params},
    error::Result,
    protocol::ProtocolInterface,
    uefi::{
        boot_services::{BootServices, StandardBootServices},
        driver_binding::UefiDriverBinding,
    },
};

/// Zero-sized marker protocol used to create a dedicated driver binding handle.
#[repr(C)]
struct UsbBusMarker;

// SAFETY: UsbBusMarker is a ZST whose GUID uniquely identifies this component.
unsafe impl ProtocolInterface for UsbBusMarker {
    const PROTOCOL_GUID: BinaryGuid = BinaryGuid::from_string("dceefc3d-ad07-4986-be64-f5ba2ed6591c");
}

/// USB bus Patina component.
///
/// When dispatched, installs a UEFI Driver Binding that produces the UsbIo protocol.
pub struct UsbBusComponent;

#[component]
impl UsbBusComponent {
    fn entry_point(self, boot_services: StandardBootServices, image_handle: params::Handle) -> Result<()> {
        let boot_services: &'static StandardBootServices = Box::leak(Box::new(boot_services));
        install_usb_bus_driver_binding(boot_services, *image_handle)
    }
}

/// Installs the USB bus driver binding using the provided boot services.
fn install_usb_bus_driver_binding<T: BootServices + Clone + 'static>(
    boot_services: &'static T,
    image_handle: efi::Handle,
) -> Result<()> {
    let (driver_binding_handle, _marker_key) =
        boot_services.install_protocol_interface(None, Box::new(UsbBusMarker))?;

    let driver = driver::UsbBusDriver::new(driver_binding_handle);

    let mut driver_binding =
        UefiDriverBinding::new_with_driver_handle(driver, image_handle, driver_binding_handle, boot_services);

    driver_binding.install().map_err(patina::error::EfiError::from)?;

    Ok(())
}

#[cfg(test)]
mod test {
    use patina::boot_services::{MockBootServices, c_ptr::CPtr};

    use super::*;

    #[test]
    fn install_usb_bus_binding_should_install_a_binding() {
        let boot_services = Box::leak(Box::new(MockBootServices::new()));

        boot_services.expect_install_protocol_interface::<UsbBusMarker, Box<UsbBusMarker>>().returning(
            |handle, protocol_interface| {
                assert_eq!(handle, None, "Expected no handle for marker protocol installation");
                Ok((0x5678 as efi::Handle, protocol_interface.metadata()))
            },
        );

        boot_services.expect_install_protocol_interface_unchecked().returning(|handle, protocol, interface| {
            if protocol == &efi::protocols::driver_binding::PROTOCOL_GUID {
                assert!(
                    handle.is_some_and(|handle| handle as usize == 0x5678),
                    "Expected correct handle for driver binding protocol"
                );
                assert!(!interface.is_null(), "Expected non-null interface for driver binding protocol");
                return Ok(0x9abc as efi::Handle);
            }
            panic!("Unexpected protocol installation: {:?}", protocol);
        });

        let mock_image_handle = 0x1234 as efi::Handle;
        install_usb_bus_driver_binding(boot_services, mock_image_handle).expect("install should succeed");
    }

    #[test]
    fn install_usb_bus_binding_handles_marker_failure() {
        let boot_services = Box::leak(Box::new(MockBootServices::new()));

        boot_services
            .expect_install_protocol_interface::<UsbBusMarker, Box<UsbBusMarker>>()
            .returning(|_, _| Err(efi::Status::OUT_OF_RESOURCES));

        let mock_image_handle = 0x1234 as efi::Handle;
        assert_eq!(
            install_usb_bus_driver_binding(boot_services, mock_image_handle),
            Err(efi::Status::OUT_OF_RESOURCES.into())
        );
    }
}
