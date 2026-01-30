use std::fmt::Display;

use crate::{cpu::mos6502::bus::Slave, errors::Error};

pub mod disks;
pub mod keyboard;
pub mod memory;
pub mod misc;
pub mod peripheral;
pub mod peripherals;
pub mod soft_switches;
pub mod speaker;
pub mod video;

pub trait SlaveAccessDetectOnly {
    fn access(&self, address: usize) -> Result<(), Error>;
}

impl<T> Slave for T
where
    T: SlaveAccessDetectOnly,
    T: Display,
{
    fn read(&self, address: usize) -> Result<u8, crate::errors::Error> {
        self.access(address)?;
        Ok(0)
    }

    fn write(&self, address: usize, _data: u8) -> Result<(), crate::errors::Error> {
        self.access(address)
    }
}
