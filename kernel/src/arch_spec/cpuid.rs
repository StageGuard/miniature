use raw_cpuid::{CpuId, CpuIdResult};
use core::fmt::{Result, Write};

use crate::logger::{FRAMEBUFFER_LOGGER};

pub fn cpuid() -> CpuId {
    // FIXME check for cpuid availability during early boot and error out if it doesn't exist.
    CpuId::with_cpuid_fn(|a, c| {
        let result = unsafe { core::arch::x86_64::__cpuid_count(a, c) };
        CpuIdResult {
            eax: result.eax,
            ebx: result.ebx,
            ecx: result.ecx,
            edx: result.edx,
        }
    })
}

pub fn cpu_info() -> Result {
    let fl_ref = FRAMEBUFFER_LOGGER.inner_exclusive_mut();
    let fl = unsafe { fl_ref.assume_init_ref() };
    let mut fl_writer = fl.writer.lock();

    let cpuid = cpuid();


    writeln!(fl_writer, "CPU Info: ");

    if let Some(info) = cpuid.get_vendor_info() {
        writeln!(fl_writer, "  Vendor: {}", info.as_str())?;
    }

    if let Some(brand) = cpuid.get_processor_brand_string() {
        writeln!(fl_writer, "  Model: {}", brand.as_str())?;
    }

    if let Some(info) = cpuid.get_processor_frequency_info() {
        writeln!(fl_writer, "  CPU Base MHz: {}", info.processor_base_frequency())?;
        writeln!(fl_writer, "  CPU Max MHz: {}", info.processor_max_frequency())?;
        writeln!(fl_writer, "  Bus MHz: {}", info.bus_frequency())?;
    }

    write!(fl_writer, "  Features:")?;

    if let Some(info) = cpuid.get_feature_info() {
        if info.has_fpu() {
            write!(fl_writer, " fpu")?
        };
        if info.has_vme() {
            write!(fl_writer, " vme")?
        };
        if info.has_de() {
            write!(fl_writer, " de")?
        };
        if info.has_pse() {
            write!(fl_writer, " pse")?
        };
        if info.has_tsc() {
            write!(fl_writer, " tsc")?
        };
        if info.has_msr() {
            write!(fl_writer, " msr")?
        };
        if info.has_pae() {
            write!(fl_writer, " pae")?
        };
        if info.has_mce() {
            write!(fl_writer, " mce")?
        };

        if info.has_cmpxchg8b() {
            write!(fl_writer, " cx8")?
        };
        if info.has_apic() {
            write!(fl_writer, " apic")?
        };
        if info.has_sysenter_sysexit() {
            write!(fl_writer, " sep")?
        };
        if info.has_mtrr() {
            write!(fl_writer, " mtrr")?
        };
        if info.has_pge() {
            write!(fl_writer, " pge")?
        };
        if info.has_mca() {
            write!(fl_writer, " mca")?
        };
        if info.has_cmov() {
            write!(fl_writer, " cmov")?
        };
        if info.has_pat() {
            write!(fl_writer, " pat")?
        };

        if info.has_pse36() {
            write!(fl_writer, " pse36")?
        };
        if info.has_psn() {
            write!(fl_writer, " psn")?
        };
        if info.has_clflush() {
            write!(fl_writer, " clflush")?
        };
        if info.has_ds() {
            write!(fl_writer, " ds")?
        };
        if info.has_acpi() {
            write!(fl_writer, " acpi")?
        };
        if info.has_mmx() {
            write!(fl_writer, " mmx")?
        };
        if info.has_fxsave_fxstor() {
            write!(fl_writer, " fxsr")?
        };
        if info.has_sse() {
            write!(fl_writer, " sse")?
        };

        if info.has_sse2() {
            write!(fl_writer, " sse2")?
        };
        if info.has_ss() {
            write!(fl_writer, " ss")?
        };
        if info.has_htt() {
            write!(fl_writer, " ht")?
        };
        if info.has_tm() {
            write!(fl_writer, " tm")?
        };
        if info.has_pbe() {
            write!(fl_writer, " pbe")?
        };

        if info.has_sse3() {
            write!(fl_writer, " sse3")?
        };
        if info.has_pclmulqdq() {
            write!(fl_writer, " pclmulqdq")?
        };
        if info.has_ds_area() {
            write!(fl_writer, " dtes64")?
        };
        if info.has_monitor_mwait() {
            write!(fl_writer, " monitor")?
        };
        if info.has_cpl() {
            write!(fl_writer, " ds_cpl")?
        };
        if info.has_vmx() {
            write!(fl_writer, " vmx")?
        };
        if info.has_smx() {
            write!(fl_writer, " smx")?
        };
        if info.has_eist() {
            write!(fl_writer, " est")?
        };

        if info.has_tm2() {
            write!(fl_writer, " tm2")?
        };
        if info.has_ssse3() {
            write!(fl_writer, " ssse3")?
        };
        if info.has_cnxtid() {
            write!(fl_writer, " cnxtid")?
        };
        if info.has_fma() {
            write!(fl_writer, " fma")?
        };
        if info.has_cmpxchg16b() {
            write!(fl_writer, " cx16")?
        };
        if info.has_pdcm() {
            write!(fl_writer, " pdcm")?
        };
        if info.has_pcid() {
            write!(fl_writer, " pcid")?
        };
        if info.has_dca() {
            write!(fl_writer, " dca")?
        };

        if info.has_sse41() {
            write!(fl_writer, " sse4_1")?
        };
        if info.has_sse42() {
            write!(fl_writer, " sse4_2")?
        };
        if info.has_x2apic() {
            write!(fl_writer, " x2apic")?
        };
        if info.has_movbe() {
            write!(fl_writer, " movbe")?
        };
        if info.has_popcnt() {
            write!(fl_writer, " popcnt")?
        };
        if info.has_tsc_deadline() {
            write!(fl_writer, " tsc_deadline_timer")?
        };
        if info.has_aesni() {
            write!(fl_writer, " aes")?
        };
        if info.has_xsave() {
            write!(fl_writer, " xsave")?
        };

        if info.has_oxsave() {
            write!(fl_writer, " xsaveopt")?
        };
        if info.has_avx() {
            write!(fl_writer, " avx")?
        };
        if info.has_f16c() {
            write!(fl_writer, " f16c")?
        };
        if info.has_rdrand() {
            write!(fl_writer, " rdrand")?
        };
    }

    if let Some(info) = cpuid.get_extended_processor_and_feature_identifiers() {
        if info.has_64bit_mode() {
            write!(fl_writer, " lm")?
        };
        if info.has_rdtscp() {
            write!(fl_writer, " rdtscp")?
        };
        if info.has_1gib_pages() {
            write!(fl_writer, " pdpe1gb")?
        };
        if info.has_execute_disable() {
            write!(fl_writer, " nx")?
        };
        if info.has_syscall_sysret() {
            write!(fl_writer, " syscall")?
        };
        if info.has_prefetchw() {
            write!(fl_writer, " prefetchw")?
        };
        if info.has_lzcnt() {
            write!(fl_writer, " lzcnt")?
        };
        if info.has_lahf_sahf() {
            write!(fl_writer, " lahf_lm")?
        };
    }

    if let Some(info) = cpuid.get_advanced_power_mgmt_info() {
        if info.has_invariant_tsc() {
            write!(fl_writer, " constant_tsc")?
        };
    }

    if let Some(info) = cpuid.get_extended_feature_info() {
        if info.has_fsgsbase() {
            write!(fl_writer, " fsgsbase")?
        };
        if info.has_tsc_adjust_msr() {
            write!(fl_writer, " tsc_adjust")?
        };
        if info.has_bmi1() {
            write!(fl_writer, " bmi1")?
        };
        if info.has_hle() {
            write!(fl_writer, " hle")?
        };
        if info.has_avx2() {
            write!(fl_writer, " avx2")?
        };
        if info.has_smep() {
            write!(fl_writer, " smep")?
        };
        if info.has_bmi2() {
            write!(fl_writer, " bmi2")?
        };
        if info.has_rep_movsb_stosb() {
            write!(fl_writer, " erms")?
        };
        if info.has_invpcid() {
            write!(fl_writer, " invpcid")?
        };
        if info.has_rtm() {
            write!(fl_writer, " rtm")?
        };
        //if info.has_qm() { write!(fl_writer, " qm")? };
        if info.has_fpu_cs_ds_deprecated() {
            write!(fl_writer, " fpu_seg")?
        };
        if info.has_mpx() {
            write!(fl_writer, " mpx")?
        };
    }

    writeln!(fl_writer)?;

    Ok(())
}
