use std::{
    fmt::{self, Display},
    sync::{Arc, RwLock},
};

use crate::errors::Error;

pub mod memory;

struct BusSlave {
    slave: Arc<dyn Slave + Send + Sync>,
    start_address: u16,
    end_address: u16,
    start_address_remap: u16,
}

struct State {
    slaves: Vec<BusSlave>,
    interceptors: Vec<Arc<dyn Interceptor + Send + Sync>>,
}

impl Display for BusSlave {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.slave.fmt(f)
    }
}
pub struct Bus {
    state: RwLock<State>,
}

impl Bus {
    pub fn new() -> Bus {
        Bus {
            state: RwLock::new(State {
                slaves: vec![],
                interceptors: vec![],
            }),
        }
    }

    fn with_slave<F, R>(&self, address: u16, mut with: F) -> R
    where
        F: FnMut(&BusSlave) -> R,
    {
        let state = self.state.read().unwrap();
        for slave in state.slaves.iter() {
            if address >= slave.start_address && address <= slave.end_address {
                return with(slave);
            }
        }
        panic!("bus error, no slave could handle {:x?}", address);
    }

    pub fn read(&self, address: u16) -> u8 {
        {
            let state = self.state.read().unwrap();
            for int in &state.interceptors {
                if let Some(byte) = int.read(address as usize).unwrap() {
                    return byte;
                }
            }
        }

        self.with_slave(address, |slave| {
            let val = slave
                .slave
                .read((address - slave.start_address_remap) as usize)
                .expect(&format!("failure reading memory at {:x?}", address));
            //trace!("read {:x?} -> {:x?}", address, val);
            val
        })
    }

    pub fn write(&self, address: u16, data: u8) {
        {
            let state = self.state.read().unwrap();
            for int in &state.interceptors {
                if let Some(_) = int.write(address as usize, data).unwrap() {
                    return;
                }
            }
        }

        self.with_slave(address, |slave| {
            slave
                .slave
                .write((address - slave.start_address_remap) as usize, data)
                .unwrap()
        });
    }

    pub fn write_u16(&self, address: u16, data: u16) {
        self.write(address, data as u8);
        self.write(address + 1, (data >> 8) as u8);
    }

    pub fn read_u16(&self, address: u16) -> u16 {
        ((self.read(address + 1) as u16) << 8) | self.read(address) as u16
    }

    pub fn add_interceptor(&self, interceptor: Arc<dyn Interceptor + Send + Sync>) {
        self.state.write().unwrap().interceptors.push(interceptor);
    }

    pub fn connect_map_insert_first(
        &self,
        start_address: usize,
        start_address_remap: usize,
        size: usize,
        slave: Arc<dyn Slave + Send + Sync>,
    ) {
        let mut state = self.state.write().unwrap();

        state.slaves.insert(
            0,
            BusSlave {
                slave,
                start_address: start_address as u16,
                start_address_remap: start_address_remap as u16,
                end_address: (start_address as usize + size - 1) as u16,
            },
        );
    }

    pub fn connect_map(
        &self,
        start_address: usize,
        start_address_remap: usize,
        size: usize,
        slave: Arc<dyn Slave + Send + Sync>,
    ) {
        let mut state = self.state.write().unwrap();

        state.slaves.push(BusSlave {
            slave,
            start_address: start_address as u16,
            start_address_remap: start_address_remap as u16,
            end_address: (start_address as usize + size - 1) as u16,
        });
    }

    pub fn connect(&self, start_address: usize, size: usize, slave: Arc<dyn Slave + Send + Sync>) {
        self.connect_map(start_address, start_address, size, slave)
    }
}

impl Display for Bus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.state.read().unwrap();
        f.write_str("\nMemory Map:\n")?;
        for slave in state.slaves.iter() {
            f.write_fmt(format_args!(
                "0x{:04x}-0x{:04x} - {}\n",
                slave.start_address, slave.end_address, slave,
            ))?;
        }
        f.write_str("\nInterceptors:\n")?;
        for interceptor in state.interceptors.iter() {
            f.write_fmt(format_args!("{}\n", interceptor))?;
        }
        f.write_str("\n")?;
        fmt::Result::Ok(())
    }
}

pub trait Interceptor: Display {
    fn read(&self, address: usize) -> Result<Option<u8>, Error>;
    fn write(&self, address: usize, data: u8) -> Result<Option<()>, Error>;
}

pub trait Slave: Display {
    fn read(&self, address: usize) -> Result<u8, Error>;
    fn write(&self, address: usize, data: u8) -> Result<(), Error>;

    // not all slaves may support this
    fn size(&self) -> Result<usize, Error> {
        Ok(0)
    }
}
