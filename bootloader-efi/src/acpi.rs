use core::ptr::NonNull;
use acpi::{AcpiHandler, PhysicalMapping};
use acpi::fadt::Fadt;
use acpi::madt::{Madt, MadtEntry};
use acpi::rsdp::Rsdp;
use log::{info, warn};
use uefi::table::{cfg::{ACPI2_GUID, ACPI_GUID}, Boot, SystemTable, Runtime};
use x86_64::instructions::port::Port;
use shared::arg::{AcpiSettings, MadtInterruptSrcOverride, MadtIoApic, MadtLocalApic, MAX_CPUS};
use shared::print_panic::PrintPanic;
use crate::read_local_apic_base;


#[derive(Clone)]
struct UefiAcpiHandler<'a>(&'a SystemTable<Boot>);

impl AcpiHandler for UefiAcpiHandler<'_> {
    unsafe fn map_physical_region<T>(&self, physical_address: usize, size: usize) -> ::acpi::PhysicalMapping<Self, T> {
        PhysicalMapping::new(
            physical_address,
            NonNull::new_unchecked(physical_address as u64 as *mut T),
            size,
            size,
            self.clone()
        )
    }

    fn unmap_physical_region<T>(_region: &::acpi::PhysicalMapping<Self, T>) {

    }
}

fn validate_rsdp(address: usize) -> core::result::Result<usize, ()> {
    // paging is not enabled at this stage; we can just read the physical address here.
    let rsdp_bytes = unsafe { core::slice::from_raw_parts(address as *const u8, core::mem::size_of::<Rsdp>()) };
    let rsdp = unsafe { (rsdp_bytes.as_ptr() as *const Rsdp).as_ref::<'static>().unwrap() };

    if rsdp.signature() != *b"RSD PTR " {
        return Err(());
    }
    let mut base_sum = 0u8;
    for base_byte in &rsdp_bytes[..20] {
        base_sum = base_sum.wrapping_add(*base_byte);
    }
    if base_sum != 0 {
        return Err(());
    }

    if rsdp.revision() == 2 {
        let mut extended_sum = 0u8;
        for byte in rsdp_bytes {
            extended_sum = extended_sum.wrapping_add(*byte);
        }

        if extended_sum != 0 {
            return Err(());
        }
    }

    let length = if rsdp.revision() == 2 { rsdp.length() as usize } else { core::mem::size_of::<Rsdp>() };

    Ok(length)
}

pub fn find_acpi_table_pointer(system_table: &SystemTable<Boot>) -> Option<(usize, usize)> {
    let config_table = system_table.config_table();

    for cfg in config_table {
        if !matches!(cfg.guid, ACPI2_GUID) && !matches!(cfg.guid, ACPI_GUID) {
            continue;
        }

        if let Ok(len) = validate_rsdp(cfg.address as usize) {
            return Some((cfg.address as usize, len))
        }
    }

    None
}

pub fn parse_acpi_table(system_table: &SystemTable<Boot>, acpi_base: usize) -> AcpiSettings {
    let handler = UefiAcpiHandler(system_table);
    let acpi_table = unsafe { ::acpi::AcpiTables::from_rsdp(handler, acpi_base) }
        .or_panic("failed to parse ACPI table from RSDP");

    let fadt = acpi_table.find_table::<Fadt>()
        .or_panic("no FADT entry in ACPI table");

    if fadt.smi_cmd_port == 0 {
        warn!("System Management Mode is not supported.");
        return Default::default()
    }

    let mut smi_serial = Port::new(fadt.smi_cmd_port as u16);
    let mut pm1a_cb_serial: Port<u16> = Port::new(fadt.pm1a_control_block().unwrap().address as u16);
    unsafe {
        smi_serial.write(fadt.acpi_enable);
        // TODO: wait for 3 seconds which do as same as linux kernel
        while pm1a_cb_serial.read() & 1 == 0 {
            core::arch::asm!("hlt");
        }
    }

    let madt = acpi_table.find_table::<Madt>()
        .or_panic("no MADT entry in ACPI table");
    let madt_entries = madt.entries();

    let mut local_apic_base: Option<usize> = None;

    let mut lapics: [MadtLocalApic; MAX_CPUS] = [Default::default(); MAX_CPUS];
    let mut lapic_count = 0;
    let mut ioapics: [MadtIoApic; MAX_CPUS] = [Default::default(); MAX_CPUS];
    let mut ioapics_count = 0;
    let mut iso: [MadtInterruptSrcOverride; MAX_CPUS] = [Default::default(); MAX_CPUS];
    let mut iso_count = 0;

    for entry in madt_entries {
        match entry {
            MadtEntry::LocalApic(local_apic) => {
                let flags = local_apic.flags;
                if flags & 3 != 0 {
                    lapics[lapic_count] = MadtLocalApic {
                        id: local_apic.apic_id,
                        processor_id: local_apic.processor_id
                    };
                    lapic_count += 1;
                } else {
                    warn!("Local APIC cannot be enabled, flag: {flags}")
                }
            },
            MadtEntry::IoApic(io_apic) => {
                ioapics[ioapics_count] = MadtIoApic {
                    id: io_apic.io_apic_id,
                    address: io_apic.io_apic_address,
                    gsi_base: io_apic.global_system_interrupt_base
                };
                ioapics_count += 1;
            }
            MadtEntry::InterruptSourceOverride(iso_entry) => {
                iso[iso_count] = MadtInterruptSrcOverride {
                    bus_source: iso_entry.bus,
                    irq_source: iso_entry.irq,
                    gsi: iso_entry.global_system_interrupt,
                    flags: iso_entry.flags
                };
                iso_count += 1;
            }
            _ => { }
        }
    }

    if lapic_count != 0 {
        local_apic_base.replace(read_local_apic_base() as usize);
    }

    AcpiSettings {
        local_apic_base: local_apic_base.unwrap_or(0),
        local_apic: lapics.clone(),
        local_apic_count: lapic_count,
        io_apic: ioapics.clone(),
        io_apic_count: ioapics_count,
        interrupt_src_override: iso.clone(),
        interrupt_src_override_count: iso_count
    }
}