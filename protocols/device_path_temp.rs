#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(dead_code)]

//! Rust ABI translation of UEFI `MdePkg/Include/Protocol/DevicePath.h`.
//! All device-path records are packed according to the UEFI specification.

use patina::BinaryGuid;
use patina::protocol::ProtocolInterface;
use r_efi::base;

pub type EfiGuid = [u8; 16];
pub type EfiPhysicalAddress = u64;
pub type EfiMacAddress = [u8; 32];
pub type EfiIpv4Address = [u8; 4];
pub type EfiIpv6Address = [u8; 16];
pub type EfiIpAddress = [u8; 16];
pub type BluetoothAddress = [u8; 6];
pub type BluetoothLeAddress = [u8; 7];

// Placeholder to get things building. Need to reconcile this with the
// below implementation at some point
unsafe impl ProtocolInterface for Protocol {
    const PROTOCOL_GUID: BinaryGuid =
        BinaryGuid::from_fields(0x09576e91, 0x6d3f, 0x11d2, 0x8e, 0x39, &[0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b]);
}

pub const PROTOCOL_GUID: base::Guid =
    base::Guid::from_fields(0x09576e91, 0x6d3f, 0x11d2, 0x8e, 0x39, &[0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b]);

pub const TYPE_HARDWARE: u8 = 0x01;
pub const TYPE_ACPI: u8 = 0x02;
pub const TYPE_MESSAGING: u8 = 0x03;
pub const TYPE_MEDIA: u8 = 0x04;
pub const TYPE_BIOS: u8 = 0x05;
pub const TYPE_END: u8 = 0x7f;

pub const END_ENTIRE_DEVICE_PATH_SUBTYPE: u8 = 0xff;
pub const END_INSTANCE_DEVICE_PATH_SUBTYPE: u8 = 0x01;

#[repr(C, packed)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Protocol {
    pub r#type: u8,
    pub sub_type: u8,
    pub length: [u8; 2],
}
pub type EfiDevicePath = Protocol;

macro_rules! packed_struct {
    ($(#[$meta:meta])* $vis:vis struct $name:ident { $($field:vis $field_name:ident : $field_type:ty),* $(,)? }) => {
        $(#[$meta])*
        #[repr(C, packed)]
        #[derive(Clone, Copy, Debug, Default)]
        $vis struct $name { $(pub $field_name: $field_type),* }
    };
}

packed_struct! { pub struct PciDevicePath {
    header: Protocol, function: u8, device: u8
} }
pub const HW_PCI_DP: u8 = 0x01;

packed_struct! { pub struct PccardDevicePath {
    header: Protocol, function_number: u8
} }
pub const HW_PCCARD_DP: u8 = 0x02;

packed_struct! { pub struct MemmapDevicePath {
    header: Protocol, memory_type: u32,
    starting_address: EfiPhysicalAddress, ending_address: EfiPhysicalAddress
} }
pub const HW_MEMMAP_DP: u8 = 0x03;

packed_struct! { pub struct VendorDevicePath {
    header: Protocol, guid: EfiGuid
} }
pub const HW_VENDOR_DP: u8 = 0x04;
pub type VendorDefinedDevicePath = VendorDevicePath;

packed_struct! { pub struct ControllerDevicePath {
    header: Protocol, controller_number: u32
} }
pub const HW_CONTROLLER_DP: u8 = 0x05;

packed_struct! { pub struct BmcDevicePath {
    header: Protocol, interface_type: u8, base_address: [u8; 8]
} }
pub const HW_BMC_DP: u8 = 0x06;

packed_struct! { pub struct AcpiHidDevicePath {
    header: Protocol, hid: u32, uid: u32
} }
pub const ACPI_DP: u8 = 0x01;

packed_struct! { pub struct AcpiExtendedHidDevicePath {
    header: Protocol, hid: u32, uid: u32, cid: u32
} }
pub const ACPI_EXTENDED_DP: u8 = 0x02;

packed_struct! { pub struct AcpiAdrDevicePath {
    header: Protocol, adr: u32
} }
pub const ACPI_ADR_DP: u8 = 0x03;

packed_struct! { pub struct AcpiNvdimmDevicePath {
    header: Protocol, nfit_device_handle: u32
} }
pub const ACPI_NVDIMM_DP: u8 = 0x04;

pub const PNP_EISA_ID_CONST: u32 = 0x41d0;
pub const PNP_EISA_ID_MASK: u32 = 0xffff;
pub const fn eisa_id(name: u32, number: u32) -> u32 {
    name | (number << 16)
}
pub const fn eisa_pnp_id(pnp_id: u32) -> u32 {
    eisa_id(PNP_EISA_ID_CONST, pnp_id)
}
pub const fn efi_pnp_id(pnp_id: u32) -> u32 {
    eisa_pnp_id(pnp_id)
}
pub const fn eisa_id_to_num(id: u32) -> u32 {
    id >> 16
}

pub const ACPI_ADR_DISPLAY_TYPE_OTHER: u32 = 0;
pub const ACPI_ADR_DISPLAY_TYPE_VGA: u32 = 1;
pub const ACPI_ADR_DISPLAY_TYPE_TV: u32 = 2;
pub const ACPI_ADR_DISPLAY_TYPE_EXTERNAL_DIGITAL: u32 = 3;
pub const ACPI_ADR_DISPLAY_TYPE_INTERNAL_DIGITAL: u32 = 4;
pub const fn acpi_display_adr(
    device_id_scheme: u32,
    head_id: u32,
    non_vga_output: u32,
    bios_can_detect: u32,
    vendor_info: u32,
    display_type: u32,
    port: u32,
    index: u32,
) -> u32 {
    ((device_id_scheme & 1) << 31)
        | ((head_id & 7) << 18)
        | ((non_vga_output & 1) << 17)
        | ((bios_can_detect & 1) << 16)
        | ((vendor_info & 0xf) << 12)
        | ((display_type & 0xf) << 8)
        | ((port & 0xf) << 4)
        | (index & 0xf)
}

packed_struct! { pub struct AtapiDevicePath {
    header: Protocol, primary_secondary: u8, slave_master: u8, lun: u16
} }
pub const MSG_ATAPI_DP: u8 = 0x01;
packed_struct! { pub struct ScsiDevicePath {
    header: Protocol, pun: u16, lun: u16
} }
pub const MSG_SCSI_DP: u8 = 0x02;
packed_struct! { pub struct FibreChannelDevicePath {
    header: Protocol, reserved: u32, wwn: u64, lun: u64
} }
pub const MSG_FIBRECHANNEL_DP: u8 = 0x03;
packed_struct! { pub struct FibreChannelExDevicePath {
    header: Protocol, reserved: u32, wwn: [u8; 8], lun: [u8; 8]
} }
pub const MSG_FIBRECHANNELEX_DP: u8 = 0x15;
packed_struct! { pub struct F1394DevicePath {
    header: Protocol, reserved: u32, guid: u64
} }
pub const MSG_1394_DP: u8 = 0x04;
packed_struct! { pub struct UsbDevicePath {
    header: Protocol, parent_port_number: u8, interface_number: u8
} }
pub const MSG_USB_DP: u8 = 0x05;
packed_struct! { pub struct UsbClassDevicePath {
    header: Protocol, vendor_id: u16, product_id: u16,
    device_class: u8, device_sub_class: u8, device_protocol: u8
} }
pub const MSG_USB_CLASS_DP: u8 = 0x0f;
packed_struct! { pub struct UsbWwidDevicePath {
    header: Protocol, interface_number: u16, vendor_id: u16, product_id: u16
} }
pub const MSG_USB_WWID_DP: u8 = 0x10;
packed_struct! { pub struct DeviceLogicalUnitDevicePath {
    header: Protocol, lun: u8
} }
pub const MSG_DEVICE_LOGICAL_UNIT_DP: u8 = 0x11;
packed_struct! { pub struct SataDevicePath {
    header: Protocol, hba_port_number: u16,
    port_multiplier_port_number: u16, lun: u16
} }
pub const MSG_SATA_DP: u8 = 0x12;
pub const SATA_HBA_DIRECT_CONNECT_FLAG: u16 = 0x8000;
packed_struct! { pub struct I2oDevicePath {
    header: Protocol, tid: u32
} }
pub const MSG_I2O_DP: u8 = 0x06;
packed_struct! { pub struct MacAddrDevicePath {
    header: Protocol, mac_address: EfiMacAddress, if_type: u8
} }
pub const MSG_MAC_ADDR_DP: u8 = 0x0b;

packed_struct! { pub struct Ipv4DevicePath {
    header: Protocol, local_ip_address: EfiIpv4Address,
    remote_ip_address: EfiIpv4Address, local_port: u16, remote_port: u16,
    protocol: u16, static_ip_address: u8, gateway_ip_address: EfiIpv4Address,
    subnet_mask: EfiIpv4Address
} }
pub const MSG_IPV4_DP: u8 = 0x0c;
packed_struct! { pub struct Ipv6DevicePath {
    header: Protocol, local_ip_address: EfiIpv6Address,
    remote_ip_address: EfiIpv6Address, local_port: u16, remote_port: u16,
    protocol: u16, ip_address_origin: u8, prefix_length: u8,
    gateway_ip_address: EfiIpv6Address
} }
pub const MSG_IPV6_DP: u8 = 0x0d;
packed_struct! { pub struct InfiniBandDevicePath {
    header: Protocol, resource_flags: u32, port_gid: [u8; 16],
    service_id: u64, target_port_id: u64, device_id: u64
} }
pub const MSG_INFINIBAND_DP: u8 = 0x09;
pub const INFINIBAND_RESOURCE_FLAG_IOC_SERVICE: u32 = 0x01;
pub const INFINIBAND_RESOURCE_FLAG_EXTENDED_BOOT_ENVIRONMENT: u32 = 0x02;
pub const INFINIBAND_RESOURCE_FLAG_CONSOLE_PROTOCOL: u32 = 0x04;
pub const INFINIBAND_RESOURCE_FLAG_STORAGE_PROTOCOL: u32 = 0x08;
pub const INFINIBAND_RESOURCE_FLAG_NETWORK_PROTOCOL: u32 = 0x10;

packed_struct! { pub struct UartDevicePath {
    header: Protocol, reserved: u32, baud_rate: u64,
    data_bits: u8, parity: u8, stop_bits: u8
} }
pub const MSG_UART_DP: u8 = 0x0e;
packed_struct! { pub struct NvdimmNamespaceDevicePath {
    header: Protocol, uuid: EfiGuid
} }
pub const NVDIMM_NAMESPACE_DP: u8 = 0x20;
packed_struct! { pub struct UartFlowControlDevicePath {
    header: Protocol, guid: EfiGuid, flow_control_map: u32
} }
pub const UART_FLOW_CONTROL_HARDWARE: u32 = 0x0000_0001;
pub const UART_FLOW_CONTROL_XON_XOFF: u32 = 0x0000_0010;

packed_struct! { pub struct SasDevicePath {
    header: Protocol, guid: EfiGuid, reserved: u32,
    sas_address: u64, lun: u64, device_topology: u16, relative_target_port: u16
} }
packed_struct! { pub struct SasExDevicePath {
    header: Protocol, sas_address: [u8; 8], lun: [u8; 8],
    device_topology: u16, relative_target_port: u16
} }
pub const MSG_SASEX_DP: u8 = 0x16;
packed_struct! { pub struct NvmeNamespaceDevicePath {
    header: Protocol, namespace_id: u32, namespace_uuid: u64
} }
pub const MSG_NVME_NAMESPACE_DP: u8 = 0x17;
packed_struct! { pub struct NvmeOfNamespaceDevicePath {
    header: Protocol, namespace_id_type: u8, namespace_id: [u8; 16]
} }
pub const MSG_NVME_OF_NAMESPACE_DP: u8 = 0x22;

packed_struct! { pub struct DnsDevicePath {
    header: Protocol, is_ipv6: u8
} }
pub const MSG_DNS_DP: u8 = 0x1f;
packed_struct! { pub struct UriDevicePath {
    header: Protocol
} }
pub const MSG_URI_DP: u8 = 0x18;
packed_struct! { pub struct UfsDevicePath {
    header: Protocol, pun: u8, lun: u8
} }
pub const MSG_UFS_DP: u8 = 0x19;
packed_struct! { pub struct SdDevicePath {
    header: Protocol, slot_number: u8
} }
pub const MSG_SD_DP: u8 = 0x1a;
packed_struct! { pub struct EmmcDevicePath {
    header: Protocol, slot_number: u8
} }
pub const MSG_EMMC_DP: u8 = 0x1d;

packed_struct! { pub struct IscsiDevicePath {
    header: Protocol, network_protocol: u16, login_option: u16,
    lun: u64, target_portal_group_tag: u16
} }
pub const MSG_ISCSI_DP: u8 = 0x13;
pub const ISCSI_LOGIN_OPTION_NO_HEADER_DIGEST: u16 = 0x0000;
pub const ISCSI_LOGIN_OPTION_HEADER_DIGEST_USING_CRC32C: u16 = 0x0002;
pub const ISCSI_LOGIN_OPTION_NO_DATA_DIGEST: u16 = 0x0000;
pub const ISCSI_LOGIN_OPTION_DATA_DIGEST_USING_CRC32C: u16 = 0x0008;
pub const ISCSI_LOGIN_OPTION_AUTHMETHOD_CHAP: u16 = 0x0000;
pub const ISCSI_LOGIN_OPTION_AUTHMETHOD_NON: u16 = 0x1000;
pub const ISCSI_LOGIN_OPTION_CHAP_BI: u16 = 0x0000;
pub const ISCSI_LOGIN_OPTION_CHAP_UNI: u16 = 0x2000;

packed_struct! { pub struct VlanDevicePath {
    header: Protocol, vlan_id: u16
} }
pub const MSG_VLAN_DP: u8 = 0x14;
packed_struct! { pub struct BluetoothDevicePath {
    header: Protocol, bd_addr: BluetoothAddress
} }
pub const MSG_BLUETOOTH_DP: u8 = 0x1b;
packed_struct! { pub struct WifiDevicePath {
    header: Protocol, ssid: [u8; 32]
} }
pub const MSG_WIFI_DP: u8 = 0x1c;
packed_struct! { pub struct BluetoothLeDevicePath {
    header: Protocol, address: BluetoothLeAddress
} }
pub const MSG_BLUETOOTH_LE_DP: u8 = 0x1e;

packed_struct! { pub struct HardDriveDevicePath {
    header: Protocol, partition_number: u32, partition_start: u64,
    partition_size: u64, signature: [u8; 16], mbr_type: u8, signature_type: u8
} }
pub const MEDIA_HARDDRIVE_DP: u8 = 0x01;
pub const MBR_TYPE_PCAT: u8 = 0x01;
pub const MBR_TYPE_EFI_PARTITION_TABLE_HEADER: u8 = 0x02;
pub const NO_DISK_SIGNATURE: u8 = 0x00;
pub const SIGNATURE_TYPE_MBR: u8 = 0x01;
pub const SIGNATURE_TYPE_GUID: u8 = 0x02;
packed_struct! { pub struct CdromDevicePath {
    header: Protocol, boot_entry: u32, partition_start: u64, partition_size: u64
} }
pub const MEDIA_CDROM_DP: u8 = 0x02;
packed_struct! { pub struct FilepathDevicePath {
    header: Protocol, path_name: [u16; 1]
} }
pub const MEDIA_FILEPATH_DP: u8 = 0x04;
pub const SIZE_OF_FILEPATH_DEVICE_PATH: usize = 4;
packed_struct! { pub struct MediaProtocolDevicePath {
    header: Protocol, protocol: EfiGuid
} }
pub const MEDIA_PROTOCOL_DP: u8 = 0x05;
packed_struct! { pub struct MediaFwVolFilePathDevicePath {
    header: Protocol, fv_file_name: EfiGuid
} }
pub const MEDIA_PIWG_FW_FILE_DP: u8 = 0x06;
packed_struct! { pub struct MediaFwVolDevicePath {
    header: Protocol, fv_name: EfiGuid
} }
pub const MEDIA_PIWG_FW_VOL_DP: u8 = 0x07;
packed_struct! { pub struct MediaRelativeOffsetRangeDevicePath {
    header: Protocol, reserved: u32, starting_offset: u64, ending_offset: u64
} }
pub const MEDIA_RELATIVE_OFFSET_RANGE_DP: u8 = 0x08;
packed_struct! { pub struct MediaRamDiskDevicePath {
    header: Protocol, starting_addr: [u32; 2], ending_addr: [u32; 2],
    type_guid: EfiGuid, instance: u16
} }
pub const MEDIA_RAM_DISK_DP: u8 = 0x09;

packed_struct! { pub struct BbsBbsDevicePath {
    header: Protocol, device_type: u16, status_flag: u16, string: [u8; 1]
} }
pub const BBS_BBS_DP: u8 = 0x01;
pub const BBS_TYPE_FLOPPY: u16 = 0x01;
pub const BBS_TYPE_HARDDRIVE: u16 = 0x02;
pub const BBS_TYPE_CDROM: u16 = 0x03;
pub const BBS_TYPE_PCMCIA: u16 = 0x04;
pub const BBS_TYPE_USB: u16 = 0x05;
pub const BBS_TYPE_EMBEDDED_NETWORK: u16 = 0x06;
pub const BBS_TYPE_BEV: u16 = 0x80;
pub const BBS_TYPE_UNKNOWN: u16 = 0xff;

// C flexible-array tails are accessed from the byte buffer after the fixed prefix.
pub const MSG_USB_WWID_SERIAL_NUMBER_OFFSET: usize = 10;
pub const MSG_ISCSI_TARGET_NAME_OFFSET: usize = 14;
pub const MSG_NVME_OF_SUBSYSTEM_NQN_OFFSET: usize = 22;
pub const MSG_DNS_SERVER_ADDRESS_OFFSET: usize = 5;
pub const MSG_URI_OFFSET: usize = 4;

#[repr(C)]
#[derive(Clone, Copy)]
pub union EfiDevPath {
    pub dev_path: Protocol,
    pub pci: PciDevicePath,
    pub pccard: PccardDevicePath,
    pub mem_map: MemmapDevicePath,
    pub vendor: VendorDevicePath,
    pub controller: ControllerDevicePath,
    pub bmc: BmcDevicePath,
    pub acpi: AcpiHidDevicePath,
    pub extended_acpi: AcpiExtendedHidDevicePath,
    pub acpi_adr: AcpiAdrDevicePath,
    pub atapi: AtapiDevicePath,
    pub scsi: ScsiDevicePath,
    pub iscsi: IscsiDevicePath,
    pub fibre_channel: FibreChannelDevicePath,
    pub fibre_channel_ex: FibreChannelExDevicePath,
    pub f1394: F1394DevicePath,
    pub usb: UsbDevicePath,
    pub sata: SataDevicePath,
    pub usb_class: UsbClassDevicePath,
    pub usb_wwid: UsbWwidDevicePath,
    pub logic_unit: DeviceLogicalUnitDevicePath,
    pub i2o: I2oDevicePath,
    pub mac_addr: MacAddrDevicePath,
    pub ipv4: Ipv4DevicePath,
    pub ipv6: Ipv6DevicePath,
    pub vlan: VlanDevicePath,
    pub infini_band: InfiniBandDevicePath,
    pub uart: UartDevicePath,
    pub uart_flow_control: UartFlowControlDevicePath,
    pub sas: SasDevicePath,
    pub sas_ex: SasExDevicePath,
    pub nvme_namespace: NvmeNamespaceDevicePath,
    pub nvme_of_namespace: NvmeOfNamespaceDevicePath,
    pub dns: DnsDevicePath,
    pub uri: UriDevicePath,
    pub bluetooth: BluetoothDevicePath,
    pub wifi: WifiDevicePath,
    pub ufs: UfsDevicePath,
    pub sd: SdDevicePath,
    pub emmc: EmmcDevicePath,
    pub hard_drive: HardDriveDevicePath,
    pub cd: CdromDevicePath,
    pub file_path: FilepathDevicePath,
    pub media_protocol: MediaProtocolDevicePath,
    pub firmware_volume: MediaFwVolDevicePath,
    pub firmware_file: MediaFwVolFilePathDevicePath,
    pub offset: MediaRelativeOffsetRangeDevicePath,
    pub ram_disk: MediaRamDiskDevicePath,
    pub bbs: BbsBbsDevicePath,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union EfiDevPathPtr {
    pub dev_path: *mut Protocol,
    pub pci: *mut PciDevicePath,
    pub pccard: *mut PccardDevicePath,
    pub mem_map: *mut MemmapDevicePath,
    pub vendor: *mut VendorDevicePath,
    pub controller: *mut ControllerDevicePath,
    pub bmc: *mut BmcDevicePath,
    pub acpi: *mut AcpiHidDevicePath,
    pub extended_acpi: *mut AcpiExtendedHidDevicePath,
    pub acpi_adr: *mut AcpiAdrDevicePath,
    pub atapi: *mut AtapiDevicePath,
    pub scsi: *mut ScsiDevicePath,
    pub iscsi: *mut IscsiDevicePath,
    pub fibre_channel: *mut FibreChannelDevicePath,
    pub fibre_channel_ex: *mut FibreChannelExDevicePath,
    pub f1394: *mut F1394DevicePath,
    pub usb: *mut UsbDevicePath,
    pub sata: *mut SataDevicePath,
    pub usb_class: *mut UsbClassDevicePath,
    pub usb_wwid: *mut UsbWwidDevicePath,
    pub logic_unit: *mut DeviceLogicalUnitDevicePath,
    pub i2o: *mut I2oDevicePath,
    pub mac_addr: *mut MacAddrDevicePath,
    pub ipv4: *mut Ipv4DevicePath,
    pub ipv6: *mut Ipv6DevicePath,
    pub vlan: *mut VlanDevicePath,
    pub infini_band: *mut InfiniBandDevicePath,
    pub uart: *mut UartDevicePath,
    pub uart_flow_control: *mut UartFlowControlDevicePath,
    pub sas: *mut SasDevicePath,
    pub sas_ex: *mut SasExDevicePath,
    pub nvme_namespace: *mut NvmeNamespaceDevicePath,
    pub nvme_of_namespace: *mut NvmeOfNamespaceDevicePath,
    pub dns: *mut DnsDevicePath,
    pub uri: *mut UriDevicePath,
    pub bluetooth: *mut BluetoothDevicePath,
    pub wifi: *mut WifiDevicePath,
    pub ufs: *mut UfsDevicePath,
    pub sd: *mut SdDevicePath,
    pub emmc: *mut EmmcDevicePath,
    pub hard_drive: *mut HardDriveDevicePath,
    pub cd: *mut CdromDevicePath,
    pub file_path: *mut FilepathDevicePath,
    pub media_protocol: *mut MediaProtocolDevicePath,
    pub firmware_volume: *mut MediaFwVolDevicePath,
    pub firmware_file: *mut MediaFwVolFilePathDevicePath,
    pub offset: *mut MediaRelativeOffsetRangeDevicePath,
    pub ram_disk: *mut MediaRamDiskDevicePath,
    pub bbs: *mut BbsBbsDevicePath,
    pub raw: *mut u8,
}
