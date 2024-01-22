use core::slice;
use log::{warn, info};
use uefi::table::{SystemTable, Boot};

use crate::mem::page_allocator::boot::allocate_zeroed_page_aligned;

#[repr(packed)]
#[derive(Clone, Copy, Debug)]
pub struct Rsdp {
    signature: [u8; 8], // b"RSD PTR "
    chksum: u8,
    oem_id: [u8; 6],
    revision: u8,
    rsdt_addr: u32,
    // the following fields are only available for ACPI 2.0, and are reserved otherwise
    length: u32,
    xsdt_addr: u64,
    extended_chksum: u8,
    _rsvd: [u8; 3],
}

fn validate_rsdp(address: usize) -> core::result::Result<usize, ()> {
    // paging is not enabled at this stage; we can just read the physical address here.
    let rsdp_bytes = unsafe { core::slice::from_raw_parts(address as *const u8, core::mem::size_of::<Rsdp>()) };
    let rsdp = unsafe { (rsdp_bytes.as_ptr() as *const Rsdp).as_ref::<'static>().unwrap() };

    if rsdp.signature != *b"RSD PTR " {
        return Err(());
    }
    let mut base_sum = 0u8;
    for base_byte in &rsdp_bytes[..20] {
        base_sum = base_sum.wrapping_add(*base_byte);
    }
    if base_sum != 0 {
        return Err(());
    }

    if rsdp.revision == 2 {
        let mut extended_sum = 0u8;
        for byte in rsdp_bytes {
            extended_sum = extended_sum.wrapping_add(*byte);
        }

        if extended_sum != 0 {
            return Err(());
        }
    }

    let length = if rsdp.revision == 2 { rsdp.length as usize } else { core::mem::size_of::<Rsdp>() };

    Ok(length)
}

pub fn find_acpi_table_pointer(system_table: &SystemTable<Boot>) -> Option<(*mut u8, usize)> {
    let config_table = system_table.config_table();

    let mut rsdps_area: [u8; 128] = [0u8; 128];
    let mut rsdps_len = 4;

    for entry in config_table {
        match validate_rsdp(entry.address as usize) {
            Ok(len) => {
                rsdps_area[0..rsdps_len].copy_from_slice(&u32::to_ne_bytes(len as u32));
                
                let ptr2 = unsafe { core::slice::from_raw_parts(entry.address as *const u8, len) };

                let total_len =  rsdps_len + ptr2.len();
                if total_len > 128 {
                    warn!("rsdp buffer is overflowed, size = {}", total_len);
                    return None
                }

                rsdps_area[rsdps_len..total_len].copy_from_slice(ptr2);
                rsdps_len += ptr2.len();

                rsdps_len += if rsdps_len & 8 == 0 { 0 } else { 8 - (rsdps_len % 8) };
                
                break;
            }
            Err(_) => warn!("Found RSDP that was not valid at {:p}", entry.address as *const u8),
        }
    }

    if ! rsdps_area.is_empty() {
        unsafe {
            // Copy to page aligned area
            let rsdps_base = allocate_zeroed_page_aligned(system_table, rsdps_area.len());
            slice::from_raw_parts_mut(rsdps_base, rsdps_len).copy_from_slice(&rsdps_area[..rsdps_len]);
            info!("acpi table: 0x{:x}, size = {}", rsdps_base as usize, rsdps_len);
            Some((rsdps_base, rsdps_area.len()))
        }
    } else {
        None
    }
}