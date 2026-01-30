use std::sync::{Arc, Mutex};

use log::{debug, error};

use crate::{
    cpu::mos6502::bus::{Bus, Slave, memory::MemoryBank},
    emulator::EmulatorProgramReader,
    errors::Error,
    targets::appleii::io::peripheral::Peripheral,
};

enum Register {
    ReadBoot = 0,
    DispatchCall = 1,
    DispatchX = 2,
    DispatchY = 3,
    SmartportAccess = 4,
    SmartportArgs = 5,
}

const MLI_COMMAND: u16 = 0x42;
const MLI_UNIT_NUMBER: u16 = 0x43;
const MLI_BUFFER: u16 = 0x44;
const MLI_BLOCK: u16 = 0x46;

const MLI_COMMAND_STATUS: u8 = 0;
const MLI_COMMAND_READ: u8 = 1;
const MLI_COMMAND_WRITE: u8 = 2;
const MLI_COMMAND_FORMAT: u8 = 3;

const MLI_IO_ERROR: u8 = 0x27;
const MLI_NO_DEVICE: u8 = 0x28;
const MLI_WRITE_PROTECTED: u8 = 0x2B;

const SMARTPORT_COMMAND_STATUS: u8 = 0;
const SMARTPORT_ERR_BAD_COMMAND: u8 = 1;
const SMARTPORT_ERR_BAD_PARAM_COUNT: u8 = 4;
const SMARTPORT_ERR_BUS_ERR: u8 = 6;
const SMARTPORT_ERR_BAD_STATUS_CODE: u8 = 0x21;

#[derive(Default)]
struct State {
    data: Vec<u8>,
    dispatch_x: u8,
    dispatch_y: u8,
    smartport_args: SmartportArgs,
}

#[derive(Debug, Default)]
struct SmartportArgs {
    command: u8,
    param_list: u16,
}

pub struct Smartport {
    rom: MemoryBank,
    state: Mutex<State>,
    bus: Arc<Bus>,
}

impl Smartport {
    pub fn new(rom: &[u8], bus: Arc<Bus>) -> Smartport {
        return Smartport {
            rom: MemoryBank::new_with_data(rom, true),
            bus,
            state: Mutex::new(State::default()),
        };
    }

    pub fn attach<R: EmulatorProgramReader>(&self, mut reader: R) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap();
        reader.read_to_end(&mut state.data)?;
        Ok(())
    }

    pub fn detach(&self) {
        self.state.lock().unwrap().data.clear();
    }
}

impl Peripheral for Smartport {
    fn read_expansion_rom(&self, _address: usize) -> Result<Option<u8>, crate::errors::Error> {
        todo!()
    }

    fn read_rom(&self, address: usize) -> Result<u8, crate::errors::Error> {
        if address == 0xfe {
            if self.state.lock().unwrap().data.len() > 0 {
                // removable, read only, single volume
                return Ok(0xd3);
            } else {
                return Ok(0);
            }
        }
        self.rom.read(address)
    }

    fn device_read(&self, address: usize) -> Result<u8, crate::errors::Error> {
        let mut state = self.state.lock().unwrap();

        if address == Register::ReadBoot as usize && state.data.len() >= 512 {
            debug!("reading boot block -> $800");
            for x in 0..512 {
                self.bus.write(0x800 + x as u16, state.data[x]);
            }
            return Ok(1 << 7);
        } else if address == Register::DispatchCall as usize {
            let command = self.bus.read(MLI_COMMAND);
            debug!("dispatch call: {:x}", command);

            // For 32MB images, we need to reduce by one block (typically unused)
            // as at 16-bits we can only support up to 65535 blocks
            let mut total_blocks = state.data.len() / 512;
            if total_blocks == 65536 {
                total_blocks -= 1;
            }
            if total_blocks > 65535 {
                error!("disk image > 32MB, not supported");
            }

            if command == MLI_COMMAND_READ {
                let unit_number = self.bus.read(MLI_UNIT_NUMBER);
                let buffer = self.bus.read_u16(MLI_BUFFER) as usize;
                let block = self.bus.read_u16(MLI_BLOCK);
                debug!(
                    "read unit={:x},buffer={:x},block={}",
                    unit_number, buffer, block
                );

                state.dispatch_x = (total_blocks & 0xff) as u8;
                state.dispatch_y = ((total_blocks >> 8) & 0xff) as u8;

                let block_offset = 512 * block as usize;
                if state.data.len() >= block_offset + 512 {
                    for x in 0..512 {
                        self.bus
                            .write((buffer + x) as u16, state.data[block_offset + x]);
                    }
                    return Ok(0);
                }
                return Ok(0);
            } else if command == MLI_COMMAND_STATUS {
                state.dispatch_x = (total_blocks & 0xff) as u8;
                state.dispatch_y = ((total_blocks >> 8) & 0xff) as u8;
                return Ok(0);
            } else if command == MLI_COMMAND_WRITE {
                let unit_number = self.bus.read(MLI_UNIT_NUMBER);
                let buffer = self.bus.read_u16(MLI_BUFFER) as usize;
                let block = self.bus.read_u16(MLI_BLOCK);

                debug!(
                    "write unit={:x},buffer={:x},block={}",
                    unit_number, buffer, block
                );

                let block_offset = 512 * block as usize;
                if state.data.len() >= block_offset + 512 {
                    for x in 0..512 {
                        state.data[x] = self.bus.read((buffer + x) as u16);
                    }
                }

                return Ok(0);
            } else if command == MLI_COMMAND_FORMAT {
                state.data.iter_mut().for_each(|v| *v = 0);
                return Ok(0);
            } else {
                error!("unsupported dispatch, command {}", command);
            }
            return Ok(MLI_IO_ERROR);
        } else if address == Register::DispatchX as usize {
            return Ok(state.dispatch_x);
        } else if address == Register::DispatchY as usize {
            return Ok(state.dispatch_y);
        } else if address == Register::SmartportAccess as usize {
            if state.smartport_args.command == SMARTPORT_COMMAND_STATUS {
                let param_count = self.bus.read(state.smartport_args.param_list);
                let unit_number = self.bus.read(state.smartport_args.param_list + 1);
                let status_pointer = self.bus.read_u16(state.smartport_args.param_list + 2);
                let status_code = self.bus.read(state.smartport_args.param_list + 4);

                debug!(
                    "Smartport status, count={},unit={:x},ptr={:x},code={}",
                    param_count, unit_number, status_pointer, status_code
                );

                if param_count != 3 {
                    return Ok(SMARTPORT_ERR_BAD_PARAM_COUNT);
                }

                // Status about the port
                if unit_number == 0 {
                    if status_code != 0 {
                        return Ok(SMARTPORT_ERR_BAD_STATUS_CODE);
                    }

                    self.bus
                        .write(status_pointer, if state.data.len() > 0 { 1 } else { 0 });
                    self.bus.write(status_pointer + 1, 1 << 6); // no interrupts
                    self.bus.write_u16(status_pointer + 2, 0);
                    self.bus.write_u16(status_pointer + 4, 0);
                    self.bus.write_u16(status_pointer + 6, 0);

                    state.dispatch_y = 0;
                    state.dispatch_x = 8;

                    return Ok(0);
                } else if unit_number == 1 {
                    if status_code == 3 {
                        let blocks = state.data.len() / 512;

                        // Block device, R/W, online, format allowed, not write protected
                        let status = 0b11111100;

                        self.bus.write(status_pointer, status);
                        self.bus.write(status_pointer + 1, (blocks & 0xff) as u8);
                        self.bus
                            .write(status_pointer + 2, ((blocks >> 8) & 0xff) as u8);
                        self.bus
                            .write(status_pointer + 3, ((blocks >> 16) & 0xff) as u8);
                        let device_name = "SMARTPORT_DEVICE";
                        self.bus.write(status_pointer + 4, device_name.len() as u8);

                        let mut pointer = status_pointer + 5;
                        for chr in device_name.chars() {
                            self.bus.write(pointer, chr as u8);
                            pointer += 1;
                        }

                        self.bus.write(pointer, 0x03); // generic scsi
                        self.bus.write(pointer + 1, 0); // subtype
                        self.bus.write_u16(pointer + 2, 1);

                        state.dispatch_y = 0;
                        state.dispatch_x = 9 + device_name.len() as u8;

                        return Ok(0);
                    } else {
                        return Ok(SMARTPORT_ERR_BAD_STATUS_CODE);
                    }
                } else {
                    error!("unknown unit number: {}", unit_number);
                }
            } else {
                debug!("unknown command: {:x}", state.smartport_args.command);
            }
            return Ok(SMARTPORT_ERR_BAD_COMMAND);
        }

        Ok(0)
    }

    fn device_write(&self, address: usize, value: u8) -> Result<(), crate::errors::Error> {
        let mut state = self.state.lock().unwrap();
        if address == Register::SmartportArgs as usize {
            let lo = self.bus.read(0x100 + ((value as u16 + 1) % 256));
            let hi = self.bus.read(0x100 + ((value as u16 + 2) % 256));

            let addr = (((hi as u16) << 8) | (lo as u16)) + 1;
            state.smartport_args = SmartportArgs {
                command: self.bus.read(addr),
                param_list: self.bus.read_u16(addr + 1),
            };
            debug!("set smartport args {:x?}", state.smartport_args);
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "Smartport"
    }

    fn reset(&self) {
        let mut state = self.state.lock().unwrap();
        state.dispatch_x = 0;
        state.dispatch_y = 0;
        state.smartport_args = Default::default();
    }
}
