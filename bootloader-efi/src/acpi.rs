use acpi::rsdp::Rsdp;
use uefi::table::{cfg::{ACPI2_GUID, ACPI_GUID}, Boot, SystemTable};


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