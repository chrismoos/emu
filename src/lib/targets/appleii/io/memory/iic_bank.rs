use std::{
    fmt::Display,
    sync::{Arc, atomic::AtomicBool},
};

use log::debug;

use crate::{
    cpu::mos6502::bus::{Slave, memory::MemoryBank},
    targets::appleii::io::soft_switches::SoftSwitches,
};

pub struct AppleIICBankSwitcher {
    rom: Arc<MemoryBank>,
    main: AtomicBool,
}

impl AppleIICBankSwitcher {
    pub fn new(_soft_switches: Arc<SoftSwitches>, rom: Arc<MemoryBank>) -> AppleIICBankSwitcher {
        AppleIICBankSwitcher {
            rom,
            main: AtomicBool::new(true),
        }
    }

    pub fn toggle_bank(&self) {
        debug!("toggle IIC ROM bank");
        let _ = self.main.fetch_update(
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
            |val| Some(!val),
        );
    }

    pub fn is_main_bank(&self) -> bool {
        self.main.load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl Display for AppleIICBankSwitcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Apple IIc Bank Switcher")
    }
}

impl Slave for AppleIICBankSwitcher {
    fn read(&self, address: usize) -> Result<u8, crate::errors::Error> {
        if self.is_main_bank() {
            self.rom.read(address)
        } else {
            self.rom.read(address + 0x4000)
        }
    }

    fn write(&self, address: usize, data: u8) -> Result<(), crate::errors::Error> {
        if self.is_main_bank() {
            self.rom.write(address, data)
        } else {
            self.rom.write(address + 0x4000, data)
        }
    }
}
