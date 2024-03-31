use core::arch::{asm, global_asm};
use core::sync::atomic::{AtomicU8, Ordering};
use x86_64::registers::control::Cr3;
use x86_64::structures::paging::PhysFrame;
use x86_64::{PhysAddr, VirtAddr};
use crate::acpi::lapic::LOCAL_APIC;
use crate::{_start_ap, AP_READY, CPU_COUNT, infohart, pause};
use crate::mem::frame_allocator::frame_alloc_n;

const TRAMPOLINE: usize = 0x8000;
// x86_64 trampoline from redox kernel
static TRAMPOLINE_DATA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/trampoline"));

pub fn setup_ap_startup(lapics: &[[u8; 2]], kernel_page_table: VirtAddr) {
    let mut lapic = unsafe { LOCAL_APIC };

    for i in 0..TRAMPOLINE_DATA.len() {
        unsafe {
            (*((TRAMPOLINE as *mut u8).add(i) as *const AtomicU8))
                .store(TRAMPOLINE_DATA[i], Ordering::SeqCst);
        }
    }

    infohart!("starting ap...");
    for &[lapic_id, cpu_id] in lapics {
        if lapic.id() as u8 == lapic_id {
            infohart!("  skipping bsp");
            continue
        }

        infohart!("  starting ap {}", cpu_id);
        CPU_COUNT.fetch_add(1, Ordering::SeqCst);

        let stack_start = frame_alloc_n(64)
            .expect("failed to allocate kernel stack for ap")
            .start_address()
            .as_u64();
        let stack_end = stack_start + 64 * 4096;

        let ap_ready = (TRAMPOLINE + 8) as *mut u64;
        let ap_cpu_id = unsafe { ap_ready.add(1) };
        let ap_page_table = unsafe { ap_ready.add(2) };
        let ap_stack_start = unsafe { ap_ready.add(3) };
        let ap_stack_end = unsafe { ap_ready.add(4) };
        let ap_code = unsafe { ap_ready.add(5) };

        unsafe {
            ap_ready.write(0);
            ap_cpu_id.write(lapic_id as u64);
            ap_page_table.write(kernel_page_table.as_u64());
            ap_stack_start.write(stack_start);
            ap_stack_end.write(stack_end);
            ap_code.write(_start_ap as u64);

            // TODO: Is this necessary (this fence)?
            core::arch::asm!("");
        };

        AP_READY.store(false, Ordering::SeqCst);

        {   // INIT
            let mut icr = 0x4500 | (lapic_id as u64) << if lapic.x2 { 32 } else { 56 };
            infohart!("    lapic {} INIT...", lapic_id);
            lapic.set_icr(icr);
        }


        {  // START IPI
            let mut icr = 0x4600 | ((TRAMPOLINE >> 12) & 0xFF) as u64;
            icr |= (lapic_id as u64) << if lapic.x2 { 32 } else { 56 };
            infohart!("    lapic {} SIPI...", lapic_id);
            lapic.set_icr(icr);
        }

        // Wait for trampoline ready
        infohart!("    lapic {} wait...", lapic_id);
        while unsafe { (*ap_ready.cast::<AtomicU8>()).load(Ordering::SeqCst) } == 0 {
            pause()
        }
        infohart!("    lapic {} trampoline...", lapic_id);
        while !AP_READY.load(Ordering::SeqCst) {
            pause()
        }
        infohart!("    lapic {} ready", lapic_id);

        unsafe {
            let cr3 = Cr3::read();
            Cr3::write(PhysFrame::containing_address(PhysAddr::new(kernel_page_table.as_u64())), cr3.1)
        }
    }
}