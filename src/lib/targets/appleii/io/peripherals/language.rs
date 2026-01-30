use std::{
    fmt::Display,
    sync::{Arc, RwLock},
};

use log::trace;

use crate::{
    cpu::mos6502::bus::{Interceptor, Slave, memory::MemoryBank},
    errors::Error,
    targets::appleii::io::{peripheral::Peripheral, soft_switches::SoftSwitches},
};

struct State {
    bank_config: BankConfig,
    bank1: MemoryBank,
    bank2: MemoryBank,
    ram: MemoryBank,
}

pub struct LanguageCard {
    state: RwLock<State>,
    soft_switches: Arc<SoftSwitches>,
}

const LANGUAGE_CARD: &[u8] =
    include_bytes!("../../../../../../resources/appleii/rom/language-card-3410020F8.bin");

impl LanguageCard {
    pub fn new(soft_switches: Arc<SoftSwitches>) -> LanguageCard {
        LanguageCard {
            state: RwLock::new(State {
                bank_config: Default::default(),
                bank1: MemoryBank::new(4096, false),
                bank2: MemoryBank::new(4096, false),
                ram: MemoryBank::new(8192, false),
            }),
            soft_switches,
        }
    }

    pub fn config(&self) -> BankConfig {
        self.state.read().unwrap().bank_config.clone()
    }

    pub fn reset(&self) {
        let mut state = self.state.write().unwrap();
        state.bank_config = BankConfig::default();
        state.bank1.clear();
        state.bank2.clear();
        self.soft_switches.set_rdlcram(false);
        self.soft_switches.set_rdlbnk2(false);
    }

    fn handle_access(&self, address: usize) -> Result<(), Error> {
        let mut state = self.state.write().unwrap();
        let data = (address & 0b1111) as u8;
        let bank = if ((data >> 3) & 1) == 1 {
            Bank::One
        } else {
            Bank::Two
        };
        let mut mode = match data & 0b11 {
            3 => Mode {
                read: Location::Ram,
                write: Location::Ram,
            },
            1 => Mode {
                read: Location::Rom,
                write: Location::Ram,
            },
            2 => Mode {
                read: Location::Rom,
                write: Location::Rom,
            },
            0 => Mode {
                read: Location::Ram,
                write: Location::Rom,
            },
            _ => Mode::default(),
        };

        if mode.write == Location::Ram {
            if state.bank_config.ram_write_pending {
                state.bank_config.ram_write_pending = false;
            } else {
                state.bank_config.ram_write_pending = true;
                mode.write = state.bank_config.mode.write.clone();
            }
        } else {
            state.bank_config.ram_write_pending = false;
        }

        trace!("update bank {:?} -> {:?}", bank, mode);
        self.soft_switches.set_rdlcram(mode.read == Location::Ram);
        self.soft_switches.set_rdlbnk2(bank == Bank::Two);
        state.bank_config.bank = bank;
        state.bank_config.mode = mode;
        Ok(())
    }
}

impl Peripheral for LanguageCard {
    fn read_expansion_rom(&self, _address: usize) -> Result<Option<u8>, crate::errors::Error> {
        //debug!("expansion read");
        Ok(None)
    }

    fn read_rom(&self, address: usize) -> Result<u8, crate::errors::Error> {
        Ok(LANGUAGE_CARD[address])
    }

    fn device_read(&self, address: usize) -> Result<u8, crate::errors::Error> {
        self.handle_access(address)?;
        Ok(0)
    }

    fn device_write(&self, address: usize, _value: u8) -> Result<(), crate::errors::Error> {
        self.state.write().unwrap().bank_config.ram_write_pending = false;
        self.handle_access(address)?;
        self.state.write().unwrap().bank_config.ram_write_pending = false;
        Ok(())
    }

    fn name(&self) -> &'static str {
        "Language Card"
    }

    fn reset(&self) {
        LanguageCard::reset(self);
    }
}

impl Display for LanguageCard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Language Card")
    }
}

impl Interceptor for LanguageCard {
    fn read(&self, address: usize) -> Result<Option<u8>, crate::errors::Error> {
        let state = self.state.read().unwrap();
        if state.bank_config.mode.read == Location::Rom {
            Ok(None)
        } else {
            if address >= 0xd000 && address <= 0xdfff {
                if state.bank_config.bank == Bank::One {
                    Ok(Some(state.bank1.read(address - 0xd000)?))
                } else {
                    Ok(Some(state.bank2.read(address - 0xd000)?))
                }
            } else if address >= 0xe000 && address <= 0xffff {
                Ok(Some(state.ram.read(address - 0xe000)?))
            } else {
                Ok(None)
            }
        }
    }

    fn write(&self, address: usize, data: u8) -> Result<Option<()>, crate::errors::Error> {
        let state = self.state.write().unwrap();
        if state.bank_config.mode.write == Location::Rom {
            Ok(None)
        } else {
            if address >= 0xd000 && address <= 0xdfff {
                if state.bank_config.bank == Bank::One {
                    state.bank1.write(address - 0xd000, data)?;
                    Ok(Some(()))
                } else {
                    state.bank2.write(address - 0xd000, data)?;
                    Ok(Some(()))
                }
            } else if address >= 0xe000 && address <= 0xffff {
                state.ram.write(address - 0xe000, data)?;
                Ok(Some(()))
            } else {
                Ok(None)
            }
        }
    }
}

#[derive(Debug, Eq, PartialEq, Clone, Copy)]
pub enum Bank {
    One,
    Two,
}

impl Default for Bank {
    fn default() -> Self {
        Bank::One
    }
}

#[derive(Debug, Eq, PartialEq, Clone, Copy)]
pub enum Location {
    Ram,
    Rom,
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct Mode {
    pub read: Location,
    pub write: Location,
}
impl Default for Mode {
    fn default() -> Self {
        Self {
            read: Location::Rom,
            write: Location::Rom,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct BankConfig {
    pub bank: Bank,
    pub mode: Mode,
    ram_write_pending: bool,
}
