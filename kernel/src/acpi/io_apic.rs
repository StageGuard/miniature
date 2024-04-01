use alloc::vec::Vec;
use core::ptr;
use core::ptr::{read_volatile, write_volatile};
use lazy_static::lazy_static;
use spin::Mutex;
use shared::arg::{MadtInterruptSrcOverride, MadtIoApic};
use shared::uni_processor::UPSafeCell;
use crate::acpi::local_apic::LOCAL_APIC;
use crate::infohart;

lazy_static! {
    static ref IOAPICS: UPSafeCell<Vec<IoApic>> = unsafe { UPSafeCell::new(Vec::new()) };
    static ref SRC_OVERRIDES: UPSafeCell<Vec<Override>> = unsafe { UPSafeCell::new(Vec::new()) };
}

pub struct IoApicRegs {
    base: u32,
}

impl IoApicRegs {
    fn ioregsel(&self) -> *const u32 {
        self.base as *const u32
    }
    fn iowin(&self) -> *const u32 {
        // offset 0x10
        unsafe { (self.base + 0x10) as *const u32 }
    }
    fn write_ioregsel(&mut self, value: u32) {
        unsafe { write_volatile(self.ioregsel() as *mut u32, value) }
    }
    fn read_iowin(&self) -> u32 {
        unsafe { read_volatile(self.iowin()) }
    }
    fn write_iowin(&mut self, value: u32) {
        unsafe { write_volatile(self.iowin() as *mut u32, value) }
    }
    fn read_reg(&mut self, reg: u8) -> u32 {
        self.write_ioregsel(reg.into());
        self.read_iowin()
    }
    fn write_reg(&mut self, reg: u8, value: u32) {
        self.write_ioregsel(reg.into());
        self.write_iowin(value);
    }
    pub fn read_ioapicid(&mut self) -> u32 {
        self.read_reg(0x00)
    }
    pub fn write_ioapicid(&mut self, value: u32) {
        self.write_reg(0x00, value);
    }
    pub fn read_ioapicver(&mut self) -> u32 {
        self.read_reg(0x01)
    }
    pub fn read_ioapicarb(&mut self) -> u32 {
        self.read_reg(0x02)
    }
    pub fn read_ioredtbl(&mut self, idx: u8) -> u64 {
        assert!(idx < 24);
        let lo = self.read_reg(0x10 + idx * 2);
        let hi = self.read_reg(0x10 + idx * 2 + 1);

        u64::from(lo) | (u64::from(hi) << 32)
    }
    pub fn write_ioredtbl(&mut self, idx: u8, value: u64) {
        assert!(idx < 24);

        let lo = value as u32;
        let hi = (value >> 32) as u32;

        self.write_reg(0x10 + idx * 2, lo);
        self.write_reg(0x10 + idx * 2 + 1, hi);
    }

    pub fn max_redirection_table_entries(&mut self) -> u8 {
        let ver = self.read_ioapicver();
        ((ver & 0x00FF_0000) >> 16) as u8
    }
    pub fn id(&mut self) -> u8 {
        let id_reg = self.read_ioapicid();
        ((id_reg & 0x0F00_0000) >> 24) as u8
    }
}
pub struct IoApic {
    regs: Mutex<IoApicRegs>,
    gsi_base: u32,
    count: u8,
}
impl IoApic {
    pub fn new(regs_base: u32, gsi_start: u32) -> Self {
        let mut regs = IoApicRegs { base: regs_base };
        let count = regs.max_redirection_table_entries();

        Self {
            regs: Mutex::new(regs),
            gsi_base: gsi_start,
            count,
        }
    }
    /// Map an interrupt vector to a physical local APIC ID of a processor (thus physical mode).
    pub fn map(&self, idx: u8, info: MapInfo) {
        self.regs.lock().write_ioredtbl(idx, info.as_raw())
    }
    pub fn set_mask(&self, gsi: u32, mask: bool) {
        let idx = (gsi - self.gsi_base) as u8;
        let mut guard = self.regs.lock();

        let mut reg = guard.read_ioredtbl(idx);
        reg &= !(1 << 16);
        reg |= u64::from(mask) << 16;
        guard.write_ioredtbl(idx, reg);
    }
}
#[repr(u8)]
#[derive(Clone, Copy, Debug)]
pub enum ApicTriggerMode {
    Edge = 0,
    Level = 1,
}
#[repr(u8)]
#[derive(Clone, Copy, Debug)]
pub enum ApicPolarity {
    ActiveHigh = 0,
    ActiveLow = 1,
}
#[repr(u8)]
#[derive(Clone, Copy, Debug)]
pub enum DestinationMode {
    Physical = 0,
    Logical = 1,
}
#[repr(u8)]
#[derive(Clone, Copy, Debug)]
pub enum DeliveryMode {
    Fixed = 0b000,
    LowestPriority = 0b001,
    Smi = 0b010,
    Nmi = 0b100,
    Init = 0b101,
    ExtInt = 0b111,
}

#[derive(Clone, Copy, Debug)]
pub struct MapInfo {
    pub dest: u8,
    pub mask: bool,
    pub trigger_mode: ApicTriggerMode,
    pub polarity: ApicPolarity,
    pub dest_mode: DestinationMode,
    pub delivery_mode: DeliveryMode,
    pub vector: u8,
}

impl MapInfo {
    pub fn as_raw(&self) -> u64 {
        assert!(self.vector >= 0x20);
        assert!(self.vector <= 0xFE);

        // TODO: Check for reserved fields.

        (u64::from(self.dest) << 56)
            | (u64::from(self.mask) << 16)
            | ((self.trigger_mode as u64) << 15)
            | ((self.polarity as u64) << 13)
            | ((self.dest_mode as u64) << 11)
            | ((self.delivery_mode as u64) << 8)
            | u64::from(self.vector)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Override {
    bus_irq: u8,
    gsi: u32,

    trigger_mode: TriggerMode,
    polarity: Polarity,
}

#[derive(Clone, Copy, Debug)]
pub enum TriggerMode {
    ConformsToSpecs,
    Edge,
    Level,
}

#[derive(Clone, Copy, Debug)]
pub enum Polarity {
    ConformsToSpecs,
    ActiveHigh,
    ActiveLow,
}

pub fn setup_io_apic(
    madt_io_apics: &[MadtIoApic],
    madt_src_overrides: &[MadtInterruptSrcOverride]
) {
    let mut ioapics = IOAPICS.inner_exclusive_mut();
    let mut overrides = SRC_OVERRIDES.inner_exclusive_mut();
    let bsp_lapic_id = unsafe { LOCAL_APIC.id() };

    for entry in madt_io_apics {
        let ioapic = IoApic::new(entry.address, entry.gsi_base);
        assert_eq!(
            ioapic.regs.lock().id(),
            entry.id,
            "mismatched ACPI MADT I/O APIC ID, and the ID reported by the I/O APIC"
        );
        ioapics.push(ioapic);
    }

    for entry in madt_src_overrides {
        let flags = entry.flags;

        overrides.push(Override {
            bus_irq: entry.irq_source,
            gsi: entry.gsi,
            polarity: match (flags & 0x0003) as u8 {
                0b00 => Polarity::ConformsToSpecs,
                0b01 => Polarity::ActiveHigh,
                0b10 => continue, // reserved
                0b11 => Polarity::ActiveLow,

                _ => unreachable!(),
            },
            trigger_mode: match ((flags & 0x000C) >> 2) as u8 {
                0b00 => TriggerMode::ConformsToSpecs,
                0b01 => TriggerMode::Edge,
                0b10 => continue, // reserved
                0b11 => TriggerMode::Level,
                _ => unreachable!(),
            }
        });
    }

    infohart!("IOAPIC count: {}, INTERRUPT_SRC_OVERRIDE count: {}", ioapics.len(), overrides.len());

    for legacy_irq in 0..16 {
        let (gsi, trigger_mode, polarity) = match overrides.iter()
            .find(|over| over.bus_irq == legacy_irq)
        {
            Some(over) => (over.gsi, over.trigger_mode, over.polarity),
            None => {
                if overrides.iter().any(|over|
                    over.gsi == u32::from(legacy_irq) && over.bus_irq != legacy_irq
                ) && !overrides.iter().any(|over|
                    over.bus_irq == legacy_irq
                ) {
                    // there's an IRQ conflict, making this legacy IRQ inaccessible.
                    continue;
                }
                (legacy_irq.into(), TriggerMode::ConformsToSpecs, Polarity::ConformsToSpecs)
            }
        };

        let target_ioapic = match ioapics.iter().find(|ia|
            gsi >= ia.gsi_base && gsi < ia.gsi_base + u32::from(ia.count)
        ) {
            Some(v) => v,
            None => {
                infohart!("Unable to find a suitable APIC for legacy IRQ {} (GSI {}). It will not be mapped.", legacy_irq, gsi);
                continue;
            }
        };

        let redir_tbl_index = (gsi - target_ioapic.gsi_base) as u8;

        let map_info = MapInfo {
            // only send to the BSP
            dest: bsp_lapic_id as u8,
            dest_mode: DestinationMode::Physical,
            delivery_mode: DeliveryMode::Fixed,
            mask: false,
            polarity: match polarity {
                Polarity::ActiveHigh => ApicPolarity::ActiveHigh,
                Polarity::ActiveLow => ApicPolarity::ActiveLow,
                Polarity::ConformsToSpecs => ApicPolarity::ActiveHigh,
            },
            trigger_mode: match trigger_mode {
                TriggerMode::Edge => ApicTriggerMode::Edge,
                TriggerMode::Level => ApicTriggerMode::Level,
                TriggerMode::ConformsToSpecs => ApicTriggerMode::Edge,
            },
            vector: 32 + legacy_irq,
        };

        target_ioapic.map(redir_tbl_index, map_info);
    }
}