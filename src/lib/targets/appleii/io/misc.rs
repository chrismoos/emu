use std::{fmt::Display, sync::RwLock};

use log::trace;

use crate::cpu::mos6502::bus::Slave;

pub const GAME_SWITCH_APPLE_KEY: usize = 0;

struct State {
    game_switch: [bool; 3],
}

pub struct MiscIo {
    state: RwLock<State>,
}

impl MiscIo {
    pub fn new() -> MiscIo {
        MiscIo {
            state: RwLock::new(State {
                game_switch: [false, false, true],
            }),
        }
    }

    pub fn update_game_switch(&self, switch: usize, state: bool) {
        if switch > 2 {
            return;
        }
        self.state.write().unwrap().game_switch[switch] = state;
    }
}

impl Display for MiscIo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Miscellaneous I/O")
    }
}

impl Slave for MiscIo {
    fn read(&self, address: usize) -> Result<u8, crate::errors::Error> {
        let state = self.state.read().unwrap();
        match address {
            x if x >= 1 && x <= 3 => {
                if state.game_switch[x - 1] {
                    return Ok(1 << 7);
                }
            }
            _ => {
                trace!("unhandled read {:x?}", address);
            }
        }
        Ok(0)
    }

    fn write(&self, address: usize, data: u8) -> Result<(), crate::errors::Error> {
        trace!("write {:x?} -> {:x?}", address, data);
        Ok(())
    }
}
