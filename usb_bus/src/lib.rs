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

#[path = "../../protocols/usb_2_host_controller.rs"]
pub(crate) mod usb_2_host_controller;
#[path = "../../protocols/device_path_temp.rs"]
pub(crate) mod device_path_temp;
pub(crate) mod usb_bus_defs;
pub(crate) mod usb_desc;
pub(crate) mod usb_enumer;
pub(crate) mod usb_hub;
pub(crate) mod usb_io_impl;
pub(crate) mod usb_utility;
pub(crate) mod driver;

//#[cfg(test)]
//pub(crate) mod test_stubs;

use r_efi::efi;
use patina::{
    component::{
        component,
        service::{
            Service,
            uefi_services::{
                driver_model::driver::DriverServices,
                event::EventServices,
                protocol::{ProtocolServices, ProtocolServicesExt},
                timer_event::TimerEventServices,
                timing::TimingServices,
                tpl::TplServices,
            },
        },
    },
    error::Result,
};

/// USB bus Patina component.
///
/// When dispatched, installs a UEFI Driver Binding that produces the UsbIo protocol.
#[derive(Default)]
pub struct UsbBusComponent;

#[component]
impl UsbBusComponent {
    /// Creates a new instance of the component.
    pub fn new() -> Self {
        Self
    }

    fn entry_point(
        self,
        protocols: Service<dyn ProtocolServices>,
        drivers: Service<dyn DriverServices>,
        tpl: Service<dyn TplServices>,
        events: Service<dyn EventServices>,
        timers: Service<dyn TimerEventServices>,
        timing: Service<dyn TimingServices>,
    ) -> Result<()> {
        let driver = driver::UsbBusDriver::new(protocols, drivers, tpl, events, timers, timing);
        let _agent = protocols.install_driver_binding(driver)?;
        Ok(())
    }
}