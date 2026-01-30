use std::{fmt::Display, sync::Arc};


use crate::{
    cpu::mos6502::bus::{Interceptor, Slave, memory::MemoryBank},
    targets::appleii::io::{
        peripherals::language::{Bank, LanguageCard, Location},
        soft_switches::SoftSwitches,
    },
};

pub struct Extended80ColumnText {
    soft_switches: Arc<SoftSwitches>,
    memory: MemoryBank,
    bank1: MemoryBank,
    bank2: MemoryBank,
    language_card: Arc<LanguageCard>,
}

impl Extended80ColumnText {
    pub fn new(
        soft_switches: Arc<SoftSwitches>,
        language_card: Arc<LanguageCard>,
    ) -> Extended80ColumnText {
        Extended80ColumnText {
            soft_switches,
            language_card,
            bank1: MemoryBank::new(0x1000, false),
            bank2: MemoryBank::new(0x1000, false),
            memory: MemoryBank::new(0x10000, false),
        }
    }

    pub fn reset(&self) {
        self.memory.clear();
        self.bank1.clear();
        self.bank2.clear();
    }
}

impl Display for Extended80ColumnText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Extended 80-Column Text Card")
    }
}

impl Interceptor for Extended80ColumnText {
    fn read(&self, address: usize) -> Result<Option<u8>, crate::errors::Error> {
        let bank_config = self.language_card.config();

        // Zero page
        if address <= 0x1ff && self.soft_switches.altzp() {
            return Ok(Some(self.memory.read(address)?));
        }

        // Bank switched memory
        if address >= 0xd000
            && address <= 0xdfff
            && self.soft_switches.altzp()
            && bank_config.mode.read == Location::Ram
        {
            if bank_config.bank == Bank::Two {
                return Ok(Some(self.bank2.read(address - 0xd000)?));
            } else {
                return Ok(Some(self.bank1.read(address - 0xd000)?));
            }
        }

        // Aux RAM high memory
        if address >= 0xe000 && self.soft_switches.altzp() && bank_config.mode.read == Location::Ram
        {
            return Ok(Some(self.memory.read(address)?));
        }

        // Display memory
        if self.soft_switches.eightystore() {
            // Text Page 1
            if address >= 0x400 && address <= 0x7ff {
                if self.soft_switches.page_two() {
                    return Ok(Some(self.memory.read(address)?));
                } else {
                    return Ok(None);
                }
            }

            if address >= 0x2000 && address <= 0x3fff && self.soft_switches.hires_mode() {
                if self.soft_switches.page_two() {
                    return Ok(Some(self.memory.read(address)?));
                } else {
                    return Ok(None);
                }
            }
        }

        // AUX RAM
        if self.soft_switches.ramrd() && address >= 0x200 && address <= 0xbfff {
            return Ok(Some(self.memory.read(address)?));
        }
        return Ok(None);
    }

    fn write(&self, address: usize, data: u8) -> Result<Option<()>, crate::errors::Error> {
        let bank_config = self.language_card.config();

        // Zero page
        if address <= 0x1ff && self.soft_switches.altzp() {
            self.memory.write(address, data)?;
            return Ok(Some(()));
        }

        // Bank switched memory
        if address >= 0xd000
            && address <= 0xdfff
            && self.soft_switches.altzp()
            && bank_config.mode.write == Location::Ram
        {
            if bank_config.bank == Bank::Two {
                self.bank2.write(address - 0xd000, data)?;
                return Ok(Some(()));
            } else {
                self.bank1.write(address - 0xd000, data)?;
                return Ok(Some(()));
            }
        }

        // Aux RAM high memory
        if address >= 0xe000
            && self.soft_switches.altzp()
            && bank_config.mode.write == Location::Ram
        {
            self.memory.write(address, data)?;
            return Ok(Some(()));
        }

        // Display memory
        if self.soft_switches.eightystore() {
            // Text Page 1
            if address >= 0x400 && address <= 0x7ff {
                if self.soft_switches.page_two() {
                    self.memory.write(address, data)?;
                    return Ok(Some(()));
                } else {
                    return Ok(None);
                }
            }

            if address >= 0x2000 && address <= 0x3fff && self.soft_switches.hires_mode() {
                if self.soft_switches.page_two() {
                    self.memory.write(address, data)?;
                    return Ok(Some(()));
                } else {
                    return Ok(None);
                }
            }
        }

        // AUX RAM
        if self.soft_switches.ramwrt() && address >= 0x200 && address <= 0xbfff {
            self.memory.write(address, data)?;
            return Ok(Some(()));
        }
        return Ok(None);
    }
}

impl Slave for Extended80ColumnText {
    fn read(&self, address: usize) -> Result<u8, crate::errors::Error> {
        Ok(self.memory.read(address)?)
    }

    fn write(&self, address: usize, data: u8) -> Result<(), crate::errors::Error> {
        self.memory.write(address, data)?;
        Ok(())
    }
}
