use std::{
    fmt::Display,
    sync::RwLock,
};

use log::warn;

use crate::{cpu::mos6502::bus::Slave, errors::Error};

pub struct MemoryBank {
    data: RwLock<Vec<u8>>,
    read_only: bool,
}

impl MemoryBank {
    pub fn new(size: usize, read_only: bool) -> MemoryBank {
        MemoryBank {
            data: RwLock::new(vec![0; size]),
            read_only,
        }
    }

    pub fn new_with_data(data: &[u8], read_only: bool) -> MemoryBank {
        MemoryBank {
            data: RwLock::new(data.to_vec()),
            read_only,
        }
    }

    pub fn clear(&self) {
        self.data.write().unwrap().iter_mut().for_each(|v| *v = 0);
    }

    pub fn size(&self) -> usize {
        self.data.read().unwrap().len()
    }

    fn check_bounds(&self, index: usize) -> Result<(), Error> {
        if index >= self.data.read().unwrap().len() {
            Err(format!(
                "memory fault, address {:x?} out of bounds (max {:x?}",
                index,
                self.size() - 1
            )
            .into())
        } else {
            Ok(())
        }
    }
}

impl Slave for MemoryBank {
    fn read(&self, address: usize) -> Result<u8, crate::errors::Error> {
        self.check_bounds(address)?;
        Ok(self.data.read().unwrap()[address])
    }

    fn write(&self, address: usize, data: u8) -> Result<(), crate::errors::Error> {
        if self.read_only {
            warn!("Attempt to write to read-only memory @ {:x}", address);
            return Ok(());
        }
        self.check_bounds(address as usize)?;
        self.data.write().unwrap()[address as usize] = data;
        Ok(())
    }

    fn size(&self) -> Result<usize, Error> {
        Ok(self.size())
    }
}

impl Display for MemoryBank {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.read_only {
            f.write_str("ROM")
        } else {
            f.write_str("RAM")
        }
    }
}
