//! USB bus enumeration interface translated from `UsbEnumer.h`.

#![allow(dead_code)]

use alloc::{
    alloc::{Layout, alloc as allocate},
    boxed::Box,
    vec::Vec,
};
use core::{cell::Cell, ffi::c_void, mem, ptr, time::Duration};
use r_efi::efi;
use r_efi::efi::{Status, Event};
use scroll::{Pread, Pwrite};
use patina::{
    component::service::uefi_services::{
        handle::Handle,
        protocol::{ProtocolPtr, ProtocolServicesExt},
        tpl::{PreviousTpl, Tpl},
    },
    error::EfiError,
    pi::{
        protocol::status_code,
        status_code::{EFI_IO_BUS_USB, EFI_IOB_PC_HOTPLUG, EFI_PROGRESS_CODE},
    },
    device_path_node,
    uefi::device_path::{
        node_defs::{DevicePathType, EndEntire, MessagingSubType},
        parse_node::DevicePathNode,
        paths::DevicePath,
    },
    protocol::ProtocolInterface,
};

use crate::usb_2_host_controller::{
    EFI_USB_SPEED_FULL, EFI_USB_SPEED_HIGH, EFI_USB_SPEED_LOW, EFI_USB_SPEED_SUPER,
    USB_PORT_STAT_CONNECTION, USB_PORT_STAT_C_CONNECTION, USB_PORT_STAT_C_ENABLE,
    USB_PORT_STAT_C_OVERCURRENT, USB_PORT_STAT_C_RESET, USB_PORT_STAT_HIGH_SPEED,
    USB_PORT_STAT_LOW_SPEED, USB_PORT_STAT_OVERCURRENT, USB_PORT_STAT_SUPER_SPEED,
    Usb2HcTransactionTranslator, UsbPortFeature, UsbPortStatus,
};
use crate::device_path_temp::EfiDevicePathProtocol;

use crate::usb_bus_defs::{
    USB_ENUM_POLL_MAXIMUM_ATTEMPTS, USB_MAX_INTERFACE, USB_SET_DEVICE_ADDRESS_STALL,
    USB_WAIT_PORT_STABLE_STALL, UsbDevice, UsbHubServices, UsbInterface, USB_INTERFACE_SIGNATURE,
};
use crate::usb_desc::{
    UsbEndpointDesc, UsbInterfaceDesc, usb_build_desc_table, usb_free_dev_desc,
    usb_get_max_packet_size0, usb_set_address, usb_set_config,
};
use crate::usb_hub::{USB_HUB_API, usb_hub_ack_hub_status, usb_is_hub_interface};
use crate::usb_io_impl::new_usb_io_protocol;
use crate::usb_utility::{
    UsbIoProtocol, usb_close_host_proto_by_child, usb_free_device_path, usb_get_current_tpl,
    usb_open_host_proto_by_child, usb_bus_is_wanted_usb_io,
};

device_path_node! {
    @[DevicePathNode(DevicePathType::Messaging, MessagingSubType::Usb)]
    @[DevicePathNodeDerive(Debug, Display)]
    #[derive(Pwrite, Pread, Clone)]
    struct UsbDevicePathNode {
        parent_port_number: u8,
        interface_number: u8,
    }
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

fn usb_hub_services(interface: &UsbInterface) -> Option<UsbHubServices> {
    let device = unsafe { interface.device.as_ref()? };
    let bus = unsafe { device.bus.as_ref()? };
    Some(bus.hub_services)
}

/// Advances a byte and bit position to the next bit.
pub fn usb_next_bit(byte: &mut u8, bit: &mut u8) {
    *bit += 1;
    if *bit > 7 {
        *byte += 1;
        *bit = 0;
    }
}

/// Initializes a hub interface for enumeration.
pub type UsbHubInit = unsafe fn(&mut UsbInterface, &UsbHubServices) -> Status;

/// Gets and acknowledges the changed status of a hub port.
pub type UsbHubGetPortStatus = unsafe fn(&mut UsbInterface, u8, &mut UsbPortStatus) -> Status;

/// Clears a hub port change notification.
pub type UsbHubClearPortChange = unsafe fn(&mut UsbInterface, u8);

/// Sets a feature on a hub port.
pub type UsbHubSetPortFeature = unsafe fn(&mut UsbInterface, u8, UsbPortFeature) -> Status;

/// Clears a feature on a hub port.
pub type UsbHubClearPortFeature = unsafe fn(&mut UsbInterface, u8, UsbPortFeature) -> Status;

/// Resets a hub port.
pub type UsbHubResetPort = unsafe fn(&mut UsbInterface, u8, &UsbHubServices) -> Status;

/// Releases a hub interface.
pub type UsbHubRelease = fn(&mut UsbInterface, &UsbHubServices) -> Status;

/// Returns the endpoint descriptor with the requested address.
pub fn usb_get_endpoint_desc(usb_if: &mut UsbInterface, endpoint_address: u8) -> Option<&mut UsbEndpointDesc> {
    let setting = unsafe { &mut *usb_if.if_setting };
    let endpoint_count = setting.descriptor.num_endpoints;

    for index in 0..endpoint_count {
        let endpoint = unsafe { &mut **setting.endpoints.add(index as usize) };
        if endpoint.descriptor.endpoint_address == endpoint_address {
            return Some(endpoint);
        }
    }

    None
}

/// Selects an alternate setting for an interface.
pub fn usb_select_setting(interface: &mut UsbInterfaceDesc, alternate: u8) -> Status {
    let mut selected_index = None;

    for index in 0..interface.num_of_setting {
        let setting = unsafe { &*interface.settings[index] };
        if setting.descriptor.alternate_setting == alternate {
            selected_index = Some(index);
            break;
        }
    }

    let Some(index) = selected_index else {
        return Status::NOT_FOUND;
    };

    interface.active_index = index;
    let setting = unsafe { &*interface.settings[index] };
    for endpoint_index in 0..setting.descriptor.num_endpoints {
        let endpoint = unsafe { &mut **setting.endpoints.add(endpoint_index as usize) };
        endpoint.toggle = 0;
    }

    Status::SUCCESS
}

/// Creates an interface object for a parsed interface descriptor.
pub fn usb_create_interface(device: &mut UsbDevice, descriptor: &mut UsbInterfaceDesc) -> Option<Box<UsbInterface>> {
    let setting = unsafe { descriptor.settings[descriptor.active_index].as_mut()? };
    let parent_interface = unsafe { device.parent_if.as_ref()? };
    let bus = unsafe { device.bus.as_ref()? };
    let services = bus.hub_services;
    let parent_path = unsafe { DevicePath::try_from_ptr(parent_interface.device_path.cast()) }.ok()?;
    let usb_node = UsbDevicePathNode {
        parent_port_number: device.parent_port,
        interface_number: setting.descriptor.interface_number,
    };
    let usb_node_size = usb_node.header().length;
    let end_node_size = EndEntire.header().length;
    let parent_prefix_size = parent_path.size().checked_sub(end_node_size)?;
    let expected_path_size = parent_path.size().checked_add(usb_node_size)?;
    let mut device_path_bytes = Vec::new();
    device_path_bytes.try_reserve_exact(expected_path_size).ok()?;
    device_path_bytes.extend_from_slice(&parent_path.as_bytes()[..parent_prefix_size]);
    device_path_bytes.resize(expected_path_size, 0);
    usb_node
        .write_into(
            &mut device_path_bytes[parent_prefix_size..parent_prefix_size + usb_node_size],
        )
        .ok()?;
    device_path_bytes[parent_prefix_size + usb_node_size..]
        .copy_from_slice(&parent_path.as_bytes()[parent_prefix_size..]);
    let device_path_bytes = device_path_bytes.into_boxed_slice();
    let device_path: Box<DevicePath> = unsafe { mem::transmute(device_path_bytes) };
    let device_path_ptr = device_path
        .as_bytes()
        .as_ptr()
        .cast_mut()
        .cast::<EfiDevicePathProtocol>();

    let mut interface = try_box_new(UsbInterface {
        signature: USB_INTERFACE_SIGNATURE as usize,
        device: device as *mut UsbDevice,
        if_desc: descriptor as *mut UsbInterfaceDesc,
        if_setting: setting,
        handle: ptr::null_mut(),
        usb_io: new_usb_io_protocol(),
        device_path: device_path_ptr.cast(),
        is_managed: Cell::new(false),
        is_hub: false,
        hub_api: None,
        num_of_port: 0,
        hub_notify: ptr::null_mut(),
        hub_ep: ptr::null_mut(),
        change_map: ptr::null_mut(),
        change_map_length: 0,
        max_speed: 0,
        poll_count: 0,
    })
    .ok()?;

    let device_path_protocol = ProtocolPtr::from_raw(device_path_ptr.cast())?;
    let handle = services
        .protocols
        .install_interface(
            None,
            EfiDevicePathProtocol::PROTOCOL_GUID,
            device_path_protocol,
        )
        .ok()?;
    interface.handle = handle.as_raw();

    let usb_io = ProtocolPtr::from_raw(ptr::from_mut(&mut interface.usb_io).cast())?;
    if services
        .protocols
        .install_interface(Some(handle), UsbIoProtocol::PROTOCOL_GUID, usb_io)
        .is_err()
    {
        if services
            .protocols
            .uninstall_interface(
                handle,
                EfiDevicePathProtocol::PROTOCOL_GUID,
                device_path_protocol,
            )
            .is_err()
        {
            let _ = Box::into_raw(device_path);
        }
        return None;
    }

    if usb_open_host_proto_by_child(
        services.protocols,
        device.bus,
        services.agent.as_raw(),
        handle.as_raw(),
    )
    .is_err()
    {
        if services
            .protocols
            .uninstall_interface(handle, UsbIoProtocol::PROTOCOL_GUID, usb_io)
            .is_err()
        {
            let _ = Box::into_raw(device_path);
            let _ = Box::into_raw(interface);
            return None;
        }
        if services.protocols.uninstall_interface(
            handle,
            EfiDevicePathProtocol::PROTOCOL_GUID,
            device_path_protocol,
        ).is_err() {
            let _ = Box::into_raw(device_path);
        }
        return None;
    }

    let _ = Box::into_raw(device_path);
    Some(interface)
}

/// Releases an interface object and its driver-owned resources.
pub fn usb_free_interface(interface: &mut UsbInterface) -> Status {
    let Some(device) = (unsafe { interface.device.as_ref() }) else {
        return Status::INVALID_PARAMETER;
    };
    let Some(bus) = (unsafe { device.bus.as_ref() }) else {
        return Status::INVALID_PARAMETER;
    };
    let services = bus.hub_services;
    let Some(handle) = Handle::from_raw(interface.handle) else {
        return Status::INVALID_PARAMETER;
    };
    let Some(usb_io) = ProtocolPtr::from_raw(ptr::from_mut(&mut interface.usb_io).cast()) else {
        return Status::INVALID_PARAMETER;
    };
    let Some(device_path) = ProtocolPtr::from_raw(interface.device_path.cast()) else {
        return Status::INVALID_PARAMETER;
    };

    let _ = usb_close_host_proto_by_child(
        services.protocols,
        device.bus,
        services.agent.as_raw(),
        handle.as_raw(),
    );

    if let Err(error) = services
        .protocols
        .uninstall_interface(handle, UsbIoProtocol::PROTOCOL_GUID, usb_io)
    {
        let _ = usb_open_host_proto_by_child(
            services.protocols,
            device.bus,
            services.agent.as_raw(),
            handle.as_raw(),
        );
        return EfiError::from(error).into();
    }

    if let Err(error) = services.protocols.uninstall_interface(
        handle,
        EfiDevicePathProtocol::PROTOCOL_GUID,
        device_path,
    ) {
        let _ = services.protocols.install_interface(
            Some(handle),
            UsbIoProtocol::PROTOCOL_GUID,
            usb_io,
        );
        let _ = usb_open_host_proto_by_child(
            services.protocols,
            device.bus,
            services.agent.as_raw(),
            handle.as_raw(),
        );
        return EfiError::from(error).into();
    }

    let Some(device_path) = (unsafe { interface.device_path.cast::<EfiDevicePathProtocol>().as_mut() }) else {
        return Status::INVALID_PARAMETER;
    };
    if let Err(status) = unsafe { usb_free_device_path(device_path) } {
        return status;
    }
    interface.device_path = ptr::null_mut();
    Status::SUCCESS
}

/// Creates a child device associated with a hub port.
pub fn usb_create_device(parent_interface: &mut UsbInterface, parent_port: u8) -> Option<Box<UsbDevice>> {
    let parent = unsafe { parent_interface.device.as_ref()? };
    try_box_new(UsbDevice {
        bus: parent.bus,
        speed: 0,
        address: 0,
        max_packet0: 8,
        dev_desc: ptr::null_mut(),
        active_config: ptr::null_mut(),
        lang_id: [0; 16],
        total_lang_id: 0,
        num_of_interface: 0,
        interfaces: [ptr::null_mut(); USB_MAX_INTERFACE],
        translator: Usb2HcTransactionTranslator::default(),
        parent_addr: parent.address,
        parent_if: parent_interface,
        parent_port,
        tier: parent.tier.saturating_add(1),
        connected: true.into(),
        disconnect_fail: false.into(),
    })
    .ok()
}

/// Frees a device and its parsed descriptor tree.
pub unsafe fn usb_free_device(mut device: Box<UsbDevice>) {
    if !device.dev_desc.is_null() {
        unsafe { usb_free_dev_desc(Box::from_raw(device.dev_desc)) };
        device.dev_desc = ptr::null_mut();
    }
}

/// Initializes hub interfaces or connects a matching interface to its driver.
pub fn usb_connect_driver(interface: &mut UsbInterface) -> Status {
    let Some(device) = (unsafe { interface.device.as_ref() }) else {
        return Status::INVALID_PARAMETER;
    };
    let Some(bus) = (unsafe { device.bus.as_ref() }) else {
        return Status::INVALID_PARAMETER;
    };
    let services = bus.hub_services;

    if unsafe { usb_is_hub_interface(interface) } {
        log::info!("usb_connect_driver: found a hub device");
        return unsafe { (USB_HUB_API.init)(interface, &services) };
    }

    if !usb_bus_is_wanted_usb_io(bus, interface) {
        return Status::SUCCESS;
    }

    let Some(handle) = Handle::from_raw(interface.handle) else {
        return Status::INVALID_PARAMETER;
    };
    let previous_tpl = usb_get_current_tpl(services.tpl);
    log::info!("usb_connect_driver: TPL before connect is {}, {:p}", previous_tpl.as_raw(), interface.handle);
    services.tpl.restore_tpl(PreviousTpl::from_raw(efi::TPL_CALLBACK));
    let result = services.drivers.connect_controller(handle, true);
    interface.is_managed.set(result.is_ok());

    log::info!("usb_connect_driver: TPL after connect is {}", usb_get_current_tpl(services.tpl).as_raw());
    debug_assert_eq!(usb_get_current_tpl(services.tpl).as_raw(), efi::TPL_CALLBACK);
    if previous_tpl.as_raw() > efi::TPL_CALLBACK {
        let previous_level = match previous_tpl.as_raw() {
            efi::TPL_NOTIFY => Tpl::Notify,
            efi::TPL_HIGH_LEVEL => Tpl::HighLevel,
            efi::TPL_CALLBACK => Tpl::Callback,
            _ => Tpl::Callback,
        };
        let _ = services.tpl.raise_tpl(previous_level);
    }

    match result {
        Ok(()) => Status::SUCCESS,
        Err(error) => EfiError::from(error).into(),
    }
}

/// Releases a hub interface or disconnects drivers managing a non-hub interface.
pub fn usb_disconnect_driver(interface: &mut UsbInterface) -> Status {
    if interface.is_hub {
        let (Some(api), Some(services)) = (interface.hub_api, usb_hub_services(interface)) else {
            return Status::INVALID_PARAMETER;
        };
        return (api.release)(interface, &services);
    }

    if !interface.is_managed.get() {
        return Status::SUCCESS;
    }

    let Some(services) = usb_hub_services(interface) else {
        return Status::INVALID_PARAMETER;
    };
    let Some(handle) = Handle::from_raw(interface.handle) else {
        return Status::INVALID_PARAMETER;
    };

    let previous_tpl = usb_get_current_tpl(services.tpl);
    log::info!("usb_disconnect_driver: old TPL is {}, {:p}", previous_tpl.as_raw(), interface.handle);
    services
        .tpl
        .restore_tpl(PreviousTpl::from_raw(efi::TPL_CALLBACK));
    let result = services.drivers.disconnect_controller(handle, None, None);
    if previous_tpl.as_raw() > efi::TPL_CALLBACK {
        let previous_level = match previous_tpl.as_raw() {
            efi::TPL_NOTIFY => Tpl::Notify,
            efi::TPL_HIGH_LEVEL => Tpl::HighLevel,
            efi::TPL_CALLBACK => Tpl::Callback,
            _ => Tpl::Callback,
        };
        log::info!("usb_disconnect_driver: TPL after disconnect is {}", usb_get_current_tpl(services.tpl).as_raw());
        debug_assert_eq!(usb_get_current_tpl(services.tpl).as_raw(), efi::TPL_CALLBACK);
        let _ = services.tpl.raise_tpl(previous_level);
    }

    match result {
        Ok(()) => {
            interface.is_managed.set(false);
            Status::SUCCESS
        }
        Err(error) => {
            EfiError::from(error).into()
        }
    }
}

/// Selects a device configuration and creates its interface objects.
pub unsafe fn usb_select_config(device: &mut UsbDevice, config_value: u8) -> Status {
    let Some(device_descriptor) = (unsafe { device.dev_desc.as_mut() }) else {
        return Status::NOT_FOUND;
    };
    let configuration_count = device_descriptor.descriptor.num_configurations as usize;
    let mut selected = None;
    for index in 0..configuration_count {
        let config = unsafe { *device_descriptor.configs.add(index) };
        if let Some(config) = unsafe { config.as_ref() } {
            if config.descriptor.configuration_value == config_value {
                selected = Some(config);
                break;
            }
        }
    }
    let Some(config) = selected else { return Status::NOT_FOUND };
    device.active_config = config as *const _ as *mut _;

    let interface_count = config.descriptor.num_interfaces as usize;
    for index in 0..interface_count {
        let descriptor = unsafe { &mut **config.interfaces.add(index) };
        if descriptor.num_of_setting == 0 {
            continue;
        }
        let alternate = unsafe { (*descriptor.settings[0]).descriptor.alternate_setting };
        let status = usb_select_setting(descriptor, alternate);
        if status != Status::SUCCESS {
            return status;
        }
        let Some(interface) = usb_create_interface(device, descriptor) else {
            device.num_of_interface = index as u8;
            return Status::OUT_OF_RESOURCES;
        };
        device.interfaces[index] = Box::into_raw(interface);
        device.num_of_interface = (index + 1) as u8;
        let _ = usb_connect_driver(unsafe { &mut *device.interfaces[index] });
    }

    Status::SUCCESS
}

/// Removes all interfaces from the active device configuration.
pub unsafe fn usb_remove_config(device: &mut UsbDevice) -> Status {
    let mut result = Status::SUCCESS;

    debug_assert!((device.num_of_interface as usize) <= USB_MAX_INTERFACE);
    for interface_ptr in device
        .interfaces
        .iter_mut()
        .take(device.num_of_interface as usize)
    {
        let Some(interface) = (unsafe { interface_ptr.as_mut() }) else {
            continue;
        };

        let mut status = usb_disconnect_driver(interface);
        if !status.is_error() {
            status = usb_free_interface(interface);
            if status.is_error() {
                let _ = usb_connect_driver(interface);
            }
        }

        if status.is_error() {
            result = status;
        } else {
            drop(unsafe { Box::from_raw(ptr::from_mut(interface)) });
            *interface_ptr = ptr::null_mut();
        }
    }

    device.active_config = ptr::null_mut();

    result
}

/// Finds a child device attached to a hub port.
pub unsafe fn usb_find_child(
    hub_interface: &UsbInterface,
    port: u8,
) -> Option<ptr::NonNull<UsbDevice>> {
    let hub_device = unsafe { hub_interface.device.as_ref()? };
    let bus = unsafe { hub_device.bus.as_ref()? };
    let max_devices = (bus.max_devices as usize).min(bus.devices.len());
    for index in 1..max_devices {
        let Some(child) = ptr::NonNull::new(bus.devices[index]) else {
            continue;
        };
        let child_ref = unsafe { child.as_ref() };
        if child_ref.parent_addr == hub_device.address && child_ref.parent_port == port {
            return Some(child);
        }
    }
    None
}

/// Removes a device and its descendants from the bus.
pub unsafe fn usb_remove_device(device: *mut UsbDevice) -> Status {
    let Some(device) = (unsafe { device.as_mut() }) else { return Status::INVALID_PARAMETER };
    let bus = device.bus;
    if bus.is_null() {
        return Status::INVALID_PARAMETER;
    }
    let max_devices = unsafe { ((*bus).max_devices as usize).min((*bus).devices.len()) };
    let mut result = Status::SUCCESS;
    for index in 1..max_devices {
        let child = unsafe { (*bus).devices[index] };
        if child.is_null() || unsafe { (*child).parent_addr } != device.address {
            continue;
        }
        let status = unsafe { usb_remove_device(child) };
        if status.is_error() {
            unsafe { (*child).disconnect_fail = true.into() };
            result = status;
        }
    }
    if result.is_error() {
        return result;
    }

    device.connected = false.into();
    let status = unsafe { usb_remove_config(device) };
    if !status.is_error() {
        let address = device.address as usize;
        if address >= max_devices || unsafe { (*bus).devices[address] != device } {
            return Status::INVALID_PARAMETER;
        }
        unsafe { (*bus).devices[address] = ptr::null_mut() };
        unsafe { usb_free_device(Box::from_raw(device)) };
    } else {
        device.disconnect_fail = true.into();
    }
    status
}

/// Enumerates and configures a newly connected device.
pub fn usb_enumerate_new_device(
    hub_interface: &mut UsbInterface,
    port: u8,
    reset_is_needed: bool,
) -> Status {
    let Some(parent) = (unsafe { hub_interface.device.as_ref() }) else {
        return Status::INVALID_PARAMETER;
    };
    let Some(bus) = (unsafe { parent.bus.as_mut() }) else {
        return Status::INVALID_PARAMETER;
    };
    let Some(hub_api) = hub_interface.hub_api else {
        return Status::INVALID_PARAMETER;
    };
    let services = bus.hub_services;

    let _ = services.timing.stall(Duration::from_micros(USB_WAIT_PORT_STABLE_STALL));
    if reset_is_needed {
        let status = unsafe { (hub_api.reset_port)(hub_interface, port, &services) };
        if status.is_error() {
            return status;
        }
    }

    let Some(mut child) = usb_create_device(hub_interface, port) else {
        return Status::OUT_OF_RESOURCES;
    };

    let mut port_state = UsbPortStatus { port_status: 0, port_change_status: 0 };
    let status = unsafe { (hub_api.get_port_status)(hub_interface, port, &mut port_state) };
    if status.is_error() {
        return status;
    }
    if port_state.port_status & USB_PORT_STAT_CONNECTION == 0 {
        return Status::NOT_FOUND;
    }

    if port_state.port_status & USB_PORT_STAT_SUPER_SPEED != 0 {
        child.speed = EFI_USB_SPEED_SUPER;
        child.max_packet0 = 512;
    } else if port_state.port_status & USB_PORT_STAT_HIGH_SPEED != 0 {
        child.speed = EFI_USB_SPEED_HIGH;
        child.max_packet0 = 64;
    } else if port_state.port_status & USB_PORT_STAT_LOW_SPEED != 0 {
        child.speed = EFI_USB_SPEED_LOW;
        child.max_packet0 = 8;
    } else {
        child.speed = EFI_USB_SPEED_FULL;
        child.max_packet0 = 8;
    }

    if (child.speed == EFI_USB_SPEED_LOW || child.speed == EFI_USB_SPEED_FULL)
        && parent.speed == EFI_USB_SPEED_HIGH
    {
        child.translator.translator_hub_address = parent.address;
        child.translator.translator_port_number = port.wrapping_add(1);
    } else {
        child.translator = parent.translator;
    }

    let max_devices = (bus.max_devices as usize).min(bus.devices.len());
    let Some(address) = (1..max_devices).find(|&address| bus.devices[address].is_null()) else {
        return Status::ACCESS_DENIED;
    };

    let status = unsafe { usb_set_address(&mut child, address as u8) };
    child.address = address as u8;
    let child = Box::into_raw(child);
    bus.devices[address] = child;
    if status.is_error() {
        return status;
    }

    let _ = services.timing.stall(Duration::from_micros(USB_SET_DEVICE_ADDRESS_STALL));
    let child = unsafe { &mut *child };

    let status = unsafe { usb_get_max_packet_size0(child) };
    if status.is_error() {
        return status;
    }
    let status = unsafe { usb_build_desc_table(child) };
    if status.is_error() {
        return status;
    }

    let Some(device_descriptor) = (unsafe { child.dev_desc.as_ref() }) else {
        return Status::DEVICE_ERROR;
    };
    let Some(configuration) = (unsafe { device_descriptor.configs.as_ref() }) else {
        return Status::DEVICE_ERROR;
    };
    let Some(configuration) = (unsafe { configuration.as_ref() }) else {
        return Status::DEVICE_ERROR;
    };
    let config_value = configuration.descriptor.configuration_value;

    let status = unsafe { usb_set_config(child, config_value) };
    if status.is_error() {
        return status;
    }
    let status = unsafe { usb_select_config(child, config_value) };
    if status.is_error() {
        return status;
    }

    let _ = services
        .protocols
        .with_protocol::<status_code::StatusCodeProtocol, _>(|protocol| {
            protocol.report_status_code(
                EFI_PROGRESS_CODE,
                EFI_IO_BUS_USB | EFI_IOB_PC_HOTPLUG,
                0,
                patina::guid::CALLER_ID.as_efi_guid(),
            )
        });

    Status::SUCCESS
}

/// Enumerates a changed hub port.
pub fn usb_enumerate_port(hub_interface: &mut UsbInterface, port: u8) -> Status {
    let Some(hub_api) = hub_interface.hub_api else {
        return Status::INVALID_PARAMETER;
    };
    let mut port_state = UsbPortStatus { port_status: 0, port_change_status: 0 };
    let mut status = unsafe { (hub_api.get_port_status)(hub_interface, port, &mut port_state) };
    if status.is_error() && status != Status::DEVICE_ERROR {
        return status;
    }

    let handled_changes = USB_PORT_STAT_C_CONNECTION
        | USB_PORT_STAT_C_ENABLE
        | USB_PORT_STAT_C_OVERCURRENT
        | USB_PORT_STAT_C_RESET;
    if port_state.port_change_status & handled_changes == 0 {
        return Status::SUCCESS;
    }

    if port_state.port_change_status & USB_PORT_STAT_C_OVERCURRENT != 0
        && port_state.port_status & USB_PORT_STAT_OVERCURRENT != 0
    {
        return Status::DEVICE_ERROR;
    }

    if let Some(child) = unsafe { usb_find_child(hub_interface, port) } {
        let _ = unsafe { usb_remove_device(child.as_ptr()) };
    }

    if port_state.port_status & USB_PORT_STAT_CONNECTION != 0 {
        let reset_is_needed = port_state.port_change_status & USB_PORT_STAT_C_RESET == 0
            || status == Status::DEVICE_ERROR;
        status = usb_enumerate_new_device(hub_interface, port, reset_is_needed);
    }

    unsafe { (hub_api.clear_port_change)(hub_interface, port) };
    status
}

/// Handles changed ports for a hub interface.
pub unsafe extern "efiapi" fn usb_hub_enumeration(_event: Event, context: *mut c_void) {
    let Some(interface) = (unsafe { (context as *mut UsbInterface).as_mut() }) else {
        return;
    };

    for port in 0..interface.num_of_port {
        if let Some(child) = unsafe { usb_find_child(interface, port) }
            && unsafe { child.as_ref() }.disconnect_fail
        {
            let _ = unsafe { usb_remove_device(child.as_ptr()) };
        }
    }

    if interface.change_map.is_null() || interface.change_map_length == 0 {
        return;
    }

    let map = unsafe {
        core::slice::from_raw_parts(interface.change_map, interface.change_map_length)
    };
    for port in 0..interface.num_of_port {
        let bit_index = port as usize + 1;
        let byte_index = bit_index / 8;
        if byte_index < map.len() && map[byte_index] & (1 << (bit_index % 8)) != 0 {
            let _ = usb_enumerate_port(interface, port);
        }
    }

    if let Some(device) = unsafe { interface.device.as_mut() } {
        let _ = unsafe { usb_hub_ack_hub_status(device) };
    }

    let map = unsafe {
        Box::from_raw(core::ptr::slice_from_raw_parts_mut(
            interface.change_map,
            interface.change_map_length,
        ))
    };
    drop(map);
    interface.change_map = ptr::null_mut();
    interface.change_map_length = 0;
}

/// Handles changed ports for the root hub.
pub fn usb_root_hub_enumeration(interface: &mut UsbInterface) {

    // MU_CHANGE [BEGIN]
    // MU_CHANGE Implement Enumeration delay
    if interface.poll_count < USB_ENUM_POLL_MAXIMUM_ATTEMPTS {
        interface.poll_count += 1;
    }
    // MU_CHANGE [END]

    for port in 0..interface.num_of_port {
        if let Some(child) = unsafe { usb_find_child(interface, port) } {
            if unsafe { child.as_ref() }.disconnect_fail {
                log::info!("usb_root_hub_enumeration: The device disconnect fails at port {} from root hub {:p}", port, interface);
                let _ = unsafe { usb_remove_device(child.as_ptr()) };
            }
        }

        let _ = usb_enumerate_port(interface, port);
    }
}
