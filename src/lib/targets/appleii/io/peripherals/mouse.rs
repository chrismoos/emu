#![allow(dead_code)]
use std::sync::{Arc, Mutex};

use eframe::egui::{InputState, Rect};
use log::{debug, error, trace};
use modular_bitfield::{bitfield, prelude::*};

use crate::{
    cpu::{
        InterruptConnection,
        mos6502::bus::{Bus, Slave as _, memory::MemoryBank},
    },
    targets::appleii::io::{peripheral::Peripheral, peripherals::scratchpad::ScratchpadRam},
};

#[bitfield]
struct Mode {
    on: bool,
    interrupt_movement: bool,
    interrupt_button: bool,
    interrupt_screen_refresh: bool,
    reserved: B4,
}

impl Default for Mode {
    fn default() -> Self {
        Mode::from_bytes([0])
    }
}

#[bitfield]
struct Status {
    reserved: bool,
    interrupt_movement: bool,
    interrupt_button: bool,
    interrupt_screen_refresh: bool,
    reserved2: bool,
    changes: bool,
    button_down_prior: bool,
    button_down: bool,
}

impl Default for Status {
    fn default() -> Self {
        Status::from_bytes([0])
    }
}

enum Register {
    InitMouse = 0,
    SetMouse = 1,
    ReadMouse = 2,
    ClearMouse = 3,
    ServeMouse = 4,
    ClampMouse = 5,
    HomeMouse = 6,
    PosMouse = 7,
    GetClamp = 8,
    BasicOutput = 9,
    BasicInput = 10,
}

enum ScratchpadData {
    LowX = 0,
    LowY = 1,
    HighX = 2,
    HighY = 3,
    Reserved = 4,
    Reserved2 = 5,
    Status = 6,
    Mode = 7,
}

struct State {
    mode: Mode,
    status: Status,
    clamp_min_x: i16,
    clamp_min_y: i16,
    clamp_max_x: i16,
    clamp_max_y: i16,
    prev_mouse_x: i16,
    prev_mouse_y: i16,
    mouse_x: i16,
    mouse_y: i16,
    prev_button_down: bool,
    button_down: bool,
    basic_string: String,
}

impl Default for State {
    fn default() -> Self {
        Self {
            mode: Default::default(),
            status: Default::default(),
            clamp_min_x: 0,
            clamp_min_y: 0,
            clamp_max_x: 1023,
            clamp_max_y: 1023,
            prev_mouse_x: 0,
            prev_mouse_y: 0,
            mouse_x: 0,
            mouse_y: 0,
            prev_button_down: false,
            basic_string: "".to_owned(),
            button_down: false,
        }
    }
}

pub struct MouseCard {
    rom: MemoryBank,
    scratchpad: ScratchpadRam,
    state: Mutex<State>,
    interrupt_connection: InterruptConnection,
    bus: Arc<Bus>,
}

// TODO apple tech note for //c clamping in aux memory holes
impl MouseCard {
    pub fn new(
        rom: &[u8],
        bus: Arc<Bus>,
        scratchpad: ScratchpadRam,
        interrupt_connection: InterruptConnection,
    ) -> MouseCard {
        MouseCard {
            rom: MemoryBank::new_with_data(rom, true),
            scratchpad,
            state: Mutex::new(State::default()),
            interrupt_connection,
            bus,
        }
    }

    pub fn signal_vbl(&self) {
        let mut state = self.state.lock().unwrap();
        if (state.mode.on()
            && ((state.mode.interrupt_movement() && state.status.interrupt_movement())
                || (state.mode.interrupt_button() && state.status.interrupt_button())))
            || state.mode.interrupt_screen_refresh()
        {
            state.status.set_interrupt_screen_refresh(true);
            self.interrupt_connection.trigger(false);
        }
    }

    pub fn process_mouse(&self, input_state: &mut InputState, display_rect: &Rect) {
        if input_state.pointer.has_pointer() {
            if let Some(pos) = input_state.pointer.latest_pos() {
                if display_rect.contains(pos) {
                    let mut state = self.state.lock().unwrap();

                    let local_x = (pos.x - display_rect.min.x) / display_rect.width();
                    let local_y = (pos.y - display_rect.min.y) / display_rect.height();
                    let mouse_x = ((local_x * ((state.clamp_max_x - state.clamp_min_x) as f32))
                        + state.clamp_min_x as f32)
                        .round() as i16;
                    let mouse_y = ((local_y * ((state.clamp_max_y - state.clamp_min_y) as f32))
                        + state.clamp_min_y as f32)
                        .round() as i16;

                    let button_changed = input_state.pointer.primary_down() != state.button_down;
                    let pointer_changed =
                        (state.mouse_x != mouse_x) || (state.mouse_y != state.mouse_y);

                    state.button_down = input_state.pointer.primary_down();
                    state.mouse_x = mouse_x;
                    state.mouse_y = mouse_y;

                    if button_changed {
                        state.status.set_interrupt_button(true);
                    }

                    if pointer_changed {
                        state.status.set_interrupt_movement(true);
                    }
                }
            }
        }
    }
}

impl Peripheral for MouseCard {
    fn read_expansion_rom(&self, address: usize) -> Result<Option<u8>, crate::errors::Error> {
        error!("read expansion ROM from mouse, invalid {:x}", address);
        return Ok(None);
    }

    fn read_rom(&self, address: usize) -> Result<u8, crate::errors::Error> {
        //debug!("read rom {:x}", address);
        self.rom.read(address)
    }

    fn device_read(&self, address: usize) -> Result<u8, crate::errors::Error> {
        let mut state = self.state.lock().unwrap();
        //debug!("read {:x}", address);

        if address == Register::ServeMouse as usize {
            if state.status.interrupt_button()
                || state.status.interrupt_movement()
                || state.status.interrupt_screen_refresh()
            {
                trace!("clear mouse interrupt");
                self.interrupt_connection.release(false);
                return Ok(1);
            }
        } else if address == Register::BasicInput as usize {
            if state.basic_string.len() == 0 {
                let status = -1;
                state.basic_string = format!("{},{},{}", state.mouse_x, state.mouse_y, status,);
                return Ok(0x8d);
            } else {
                let next = state.basic_string.remove(0);
                return Ok(next as u8 | 0x80);
            }
        }
        Ok(0)
    }

    fn device_write(&self, address: usize, value: u8) -> Result<(), crate::errors::Error> {
        let mut state = self.state.lock().unwrap();
        //debug!("write {:x}", address);
        if address == Register::SetMouse as usize {
            state.mode = Mode::from_bytes([value]);
            debug!(
                "SetMouse: {:x}, interrupts (button={}, movement={}, vbl={})",
                value,
                state.mode.interrupt_button(),
                state.mode.interrupt_movement(),
                state.mode.interrupt_screen_refresh()
            );
        } else if address == Register::ClampMouse as usize {
            debug!("ClampMouse: {:x}", value);
            let min = (((self.bus.read(0x578) as u16) << 8) | (self.bus.read(0x478) as u16)) as i16;
            let max = (((self.bus.read(0x5f8) as u16) << 8) | (self.bus.read(0x4f8) as u16)) as i16;
            if value == 0 {
                debug!(
                    "Mouse clamp X updated from {}..{} to {}..{}",
                    state.clamp_min_x, state.clamp_max_x, min, max
                );
                state.clamp_min_x = min;
                state.clamp_max_x = max;
            } else {
                debug!(
                    "Mouse clamp Y updated from {}..{} to {}..{}",
                    state.clamp_min_y, state.clamp_max_y, min, max
                );
                state.clamp_min_y = min;
                state.clamp_max_y = max;
            }
        } else if address == Register::InitMouse as usize {
            debug!("InitMouse");
            *state = Default::default();
        } else if address == Register::HomeMouse as usize {
            debug!("HomeMouse");
            state.mouse_x = state.clamp_min_x;
            state.mouse_y = state.clamp_min_y;
        } else if address == Register::PosMouse as usize {
            let x = ((self.scratchpad.read(2) as u16) << 8) | (self.scratchpad.read(0) as u16);
            let y = ((self.scratchpad.read(3) as u16) << 8) | (self.scratchpad.read(1) as u16);
            //debug!("PosMouse {},{}", x, y);
            state.mouse_x = x as i16;
            state.mouse_y = y as i16;
        } else if address == Register::ReadMouse as usize {
            //debug!("ReadMouse {}/{}", state.mouse_x, state.mouse_y);
            let changes =
                (state.prev_mouse_x != state.mouse_x) || (state.prev_mouse_y != state.mouse_y);
            state.status.set_changes(changes);

            let button_prior = state.prev_button_down;
            let button_down = state.button_down;
            state.status.set_button_down_prior(button_prior);
            state.status.set_button_down(button_down);

            self.scratchpad.write(0, (state.mouse_x & 0xff) as u8);
            self.scratchpad.write(1, (state.mouse_y & 0xff) as u8);
            self.scratchpad
                .write(2, ((state.mouse_x >> 8) & 0xff) as u8);
            self.scratchpad
                .write(3, ((state.mouse_y >> 8) & 0xff) as u8);
            self.scratchpad.write(6, state.status.bytes[0]);
            self.scratchpad.write(7, state.mode.bytes[0]);

            state.prev_button_down = state.button_down;
            state.prev_mouse_x = state.mouse_x;
            state.prev_mouse_y = state.mouse_y;

            // reading the mouse clear's the interrupt flags
            state.status.set_interrupt_button(false);
            state.status.set_interrupt_movement(false);
            state.status.set_interrupt_screen_refresh(false);

            // TODO - update BIS with current/prior
        } else if address == Register::ClearMouse as usize {
            debug!("ClearMouse");
            state.mouse_x = 0;
            state.mouse_y = 0;
            self.scratchpad.write(0, 0);
            self.scratchpad.write(1, 0);
            self.scratchpad.write(2, 0);
            self.scratchpad.write(3, 0);
        } else if address == Register::GetClamp as usize {
            // todo
            debug!("get clamp");
        } else if address == Register::BasicOutput as usize {
            debug!("basic output {:x}", value);
            state.mode.set_on(true);
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "Mouse Card"
    }

    fn reset(&self) {
        *self.state.lock().unwrap() = State::default();
    }
}
