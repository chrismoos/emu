use std::sync::Arc;


use crate::cpu::mos6502::bus::Bus;

const SCRATCH_BASE: usize = 0x478;

/// Used for a peripheral to access scratchpad RAM
pub struct ScratchpadRam {
    bus: Arc<Bus>,
    slot: usize,
}

impl ScratchpadRam {
    pub fn new(bus: Arc<Bus>, slot: usize) -> ScratchpadRam {
        ScratchpadRam { bus, slot }
    }

    pub fn read(&self, index: usize) -> u8 {
        assert!(index <= 7);
        self.bus
            .read((SCRATCH_BASE + (index * 0x80) + self.slot) as u16)
    }

    pub fn write(&self, index: usize, value: u8) {
        assert!(index <= 7);
        self.bus
            .write((SCRATCH_BASE + (index * 0x80) + self.slot) as u16, value)
    }
}
