use std::{
    fmt::{self, Display},
    sync::{Arc, Mutex},
};

use log::{debug, trace};

use crate::{
    cpu::mos6502::bus::Slave,
    errors::Error,
    targets::appleii::io::soft_switches::SoftSwitches,
};

struct State {
    peripherals: [Option<Arc<dyn Peripheral + Send + Sync>>; 8],

    active_slot_expansion_rom: Option<usize>,
    c3_rom_enabled: bool,
    cx_rom_enabled: bool,

    intc8rom: bool,
}

pub struct PeripheralManager {
    state: Mutex<State>,
    soft_switches: Arc<SoftSwitches>,

    // Used to enable Apple IIe I/O memory switching
    internal_rom: Option<Arc<dyn Slave + Send + Sync>>,
}

impl PeripheralManager {
    pub fn new(
        soft_switches: Arc<SoftSwitches>,
        internal_rom: Option<Arc<dyn Slave + Send + Sync>>,
    ) -> PeripheralManager {
        if internal_rom.is_some() {
            debug!("I/O memory switching enabled.");
        }

        PeripheralManager {
            state: Mutex::new(State {
                active_slot_expansion_rom: None,
                peripherals: [const { None }; 8],
                intc8rom: false,
                c3_rom_enabled: false,
                cx_rom_enabled: false,
            }),
            soft_switches,
            internal_rom,
        }
    }

    pub fn reset(&self) {
        let mut state = self.state.lock().unwrap();
        for p in &mut state.peripherals {
            if let Some(p) = p {
                p.reset();
            }
        }

        state.intc8rom = false;
    }

    pub fn is_assigned(&self, index: usize) -> bool {
        if index < 8 {
            return self.state.lock().unwrap().peripherals[index].is_some();
        }
        true
    }

    pub fn unassign(&self, index: usize) {
        if index < 8 {
            self.state.lock().unwrap().peripherals[index] = None;
        }
    }

    pub fn assign(
        &self,
        index: usize,
        peripheral: Arc<dyn Peripheral + Send + Sync>,
    ) -> Result<(), Error> {
        if index > 7 {
            return Err("invalid index".into());
        }

        let mut state = self.state.lock().unwrap();

        if state.peripherals[index].is_some() {
            return Err(format!("slot {} is already occupied", index).into());
        }

        state.peripherals[index] = Some(peripheral);

        Ok(())
    }

    fn get_device_addr(&self, index: usize) -> usize {
        0x80 + (index * 0x10)
    }
}

impl Display for PeripheralManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PeripheralManager\n")?;
        let s = self.state.lock().unwrap();
        for x in 0..s.peripherals.len() {
            f.write_fmt(format_args!("\tSlot {} - ", x))?;
            if let Some(p) = &s.peripherals[x] {
                f.write_fmt(format_args!("{}", p.name()))?;
            } else {
                f.write_str("None")?;
            }
            f.write_str("\n")?
        }
        fmt::Result::Ok(())
    }
}

impl Slave for PeripheralManager {
    fn read(&self, address: usize) -> Result<u8, crate::errors::Error> {
        let mut state = self.state.lock().unwrap();
        if address >= 0x300 && address <= 0x3ff {
            if !self.soft_switches.c3rom_slot() {
                state.intc8rom = true;
            }

            if let Some(rom) = &self.internal_rom {
                if !self.soft_switches.c3rom_slot() || self.soft_switches.intcxrom() {
                    return rom.read(address);
                }
            }
        }

        if address >= 0x100 && address <= 0xfff {
            if let Some(rom) = &self.internal_rom {
                if self.soft_switches.intcxrom() {
                    return rom.read(address);
                }
            }
        }

        if address == 0xfff {
            trace!("clear expansion ROM enable");
            state.active_slot_expansion_rom = None;
            state.intc8rom = false;
            return Ok(0);
        }

        if address >= 0x800 && address < 0x1000 && state.intc8rom {
            if let Some(rom) = &self.internal_rom {
                return rom.read(address);
            }
        }

        for (idx, peripheral) in state.peripherals.iter().enumerate() {
            if let Some(peripheral) = peripheral {
                if address >= 0x800
                    && address < 0x1000
                    && state.active_slot_expansion_rom == Some(idx)
                {
                    // TODO - update current peripherals to support this now
                    match peripheral.read_expansion_rom(address - 0x800)? {
                        Some(data) => return Ok(data),
                        None => {
                            debug!("can't read expansion rom on slot {}, got none", idx);
                            if let Some(rom) = &self.internal_rom {
                                return rom.read(address);
                            }
                        }
                    }
                }

                if idx > 0 {
                    let base_rom = idx * 0x100;
                    if address >= base_rom && address < base_rom + 0x100 {
                        let result = Ok(peripheral.read_rom(address - base_rom)?);
                        state.active_slot_expansion_rom = Some(idx);
                        return result;
                    }
                }

                let base_device_select = self.get_device_addr(idx);
                if address >= base_device_select && address < base_device_select + 0x10 {
                    return peripheral.device_read(address - base_device_select);
                }
            }
        }

        debug!("read unhandled {:x}", address);
        Ok(0)
    }

    fn write(&self, address: usize, data: u8) -> Result<(), crate::errors::Error> {
        let mut state = self.state.lock().unwrap();

        if address == 0xfff {
            trace!("clear expansion ROM enable");
            state.active_slot_expansion_rom = None;
            state.intc8rom = false;
            return Ok(());
        }
        for (idx, peripheral) in state.peripherals.iter().enumerate() {
            if let Some(peripheral) = peripheral {
                let base_device_select = self.get_device_addr(idx);
                if address >= base_device_select && address < base_device_select + 0x10 {
                    peripheral.device_write(address - base_device_select, data)?;
                    return Ok(());
                }
            }
        }

        debug!("write unhandled {:x} -> {:x}", address, data);
        Ok(())
    }
}

pub trait Peripheral {
    // 2K Expansion ROM
    fn read_expansion_rom(&self, address: usize) -> Result<Option<u8>, Error>;

    // 256 byte ROM
    fn read_rom(&self, address: usize) -> Result<u8, Error>;

    fn device_read(&self, address: usize) -> Result<u8, Error>;
    fn device_write(&self, address: usize, value: u8) -> Result<(), Error>;

    fn name(&self) -> &'static str;

    fn reset(&self);
}
