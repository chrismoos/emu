#![allow(dead_code)]

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use log::{debug, error, trace};
use modular_bitfield::{bitfield, prelude::B6};

use crate::{
    clock::{Clock, ClockInstant},
    cpu::mos6502::bus::{Slave, memory::MemoryBank},
    errors::Error,
    targets::appleii::io::{disks::FloppyDiskReader, peripheral::Peripheral},
};

const REGISTER_MOTOR_OFF: usize = 0x08;
const REGISTER_MOTOR_ON: usize = 0x09;
const REGISTER_SELECT_DRIVE_1: usize = 0x0a;
const REGISTER_SELECT_DRIVE_2: usize = 0x0b;
const REGISTER_Q6L: usize = 0x0c;
const REGISTER_Q6H: usize = 0x0d;
const REGISTER_Q7L: usize = 0x0e;
const REGISTER_Q7H: usize = 0x0f;

const PHASE_MAX: f32 = 40.0;

#[bitfield]
struct Mode {
    latch: bool,
    asynchronous: bool,
    disable_timer: bool,
    fast_mode: bool,
    clock_8_mhz: bool,
    test_mode: bool,
    mz_reset: bool,
    reserved: bool,
}

impl Default for Mode {
    fn default() -> Self {
        Self::from_bytes([0])
    }
}

#[bitfield]
struct Status {
    latch: bool,
    asynchronous: bool,
    disable_timer: bool,
    fast_mode: bool,
    clock_8_mhz: bool,
    enable2: bool,
    mz: bool,
    sense_high: bool,
}

impl Default for Status {
    fn default() -> Self {
        Self::from_bytes([0])
    }
}

#[bitfield]
struct Handshake {
    reserved: B6,
    write_state: bool,
    write_buffer_ready: bool,
}

impl Default for Handshake {
    fn default() -> Self {
        Self::from_bytes([0])
    }
}

struct State {
    shift_register: u8,
    data_register: u8,

    q6: bool,
    q7: bool,

    last_instant: ClockInstant,
    data_valid_until: ClockInstant,

    current_phase: [bool; 4],
    absolute_phase: f32,
    motor_spinning: bool,
    motor_off_instant: ClockInstant,
    disk: Option<Box<dyn FloppyDiskReader + Send + Sync>>,

    status: Status,

    #[allow(dead_code)]
    handshake: Handshake,
}

impl State {
    pub fn initial(clock: &dyn Clock) -> State {
        State {
            q6: false,
            q7: false,
            disk: None,
            motor_spinning: false,
            data_valid_until: clock.elapsed(),
            data_register: 0,
            shift_register: 0,
            motor_off_instant: clock.elapsed(),
            last_instant: clock.elapsed(),
            current_phase: [false; 4],
            absolute_phase: 40.0,
            handshake: Handshake::default(),
            status: Status::default(),
        }
    }
}

pub struct DiskIIPeripheral {
    state: Mutex<State>,
    clock: Arc<dyn Clock + Send + Sync>,
    rom: Arc<MemoryBank>,
}

impl DiskIIPeripheral {
    pub fn new(clock: Arc<dyn Clock + Send + Sync>, rom: Arc<MemoryBank>) -> DiskIIPeripheral {
        return DiskIIPeripheral {
            state: Mutex::new(State::initial(clock.as_ref())),
            rom,
            clock,
        };
    }

    pub fn reset(&self) {
        let mut state = self.state.lock().unwrap();
        let disk = state.disk.take();
        *state = State::initial(self.clock.as_ref());

        if let Some(disk) = disk {
            if let Err(e) = disk.reset() {
                error!("failed to reset disk: {:?}", e);
            }
            state.disk = Some(disk);
        }
    }

    pub fn attach(&self, disk: Box<dyn FloppyDiskReader + Send + Sync>) {
        self.state.lock().unwrap().disk = Some(disk);
    }

    pub fn detach(&self) {
        self.state.lock().unwrap().disk = None;
    }

    fn update_phase(&self, phase: usize, on: bool) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap();
        assert!(phase <= 3);

        // no change
        if state.current_phase[phase] == on {
            return Ok(());
        }

        // 3 -> 4 phase transition? cancel?
        // 4 -> 3 phase transition? slight movement?

        // get adjacent phase
        let next = if phase == 3 { 0 } else { phase + 1 };
        let prev = if phase == 0 { 3 } else { phase - 1 };

        let mut change = 0.0;
        if on && state.current_phase[next] {
            change = -0.25;
        } else if !on && state.current_phase[next] {
            change = 0.25;
        } else if on && state.current_phase[prev] {
            change = 0.25;
        } else if !on && state.current_phase[prev] {
            change = -0.25;
        } else if on {
            //let phase_new = (phase as f32) / 4.0;

            let current_phase_pos = state.absolute_phase % 2.0;
            change = match phase {
                0 => match current_phase_pos {
                    0.5 => -0.5,
                    1.0 => 1.0, // indeterminate?
                    1.5 => 0.5,
                    _ => 0.0,
                },
                1 => match current_phase_pos {
                    0.0 => 0.5,
                    1.0 => -0.5,
                    1.5 => 1.0,
                    _ => 0.0,
                },
                2 => match current_phase_pos {
                    0.5 => 0.5,
                    0.0 => 1.0,
                    1.5 => -0.5,
                    _ => 0.0,
                },
                3 => match current_phase_pos {
                    0.0 => -0.5,
                    1.0 => 0.5,
                    0.5 => 1.0,
                    _ => 0.0,
                },
                _ => 0.0,
            };
        }

        trace!(
            "Phase {} from {} -> {}, Absolute: {} -> {}, Phase State {:?}",
            phase,
            state.current_phase[phase],
            on,
            state.absolute_phase,
            state.absolute_phase + change,
            state.current_phase,
        );

        state.current_phase[phase] = on;
        state.absolute_phase += change;
        if state.absolute_phase >= PHASE_MAX {
            state.absolute_phase = PHASE_MAX;
        } else if state.absolute_phase <= 0.0 {
            state.absolute_phase = 0.0;
        }

        let absolute_phase = state.absolute_phase as f32;
        if let Some(disk) = &mut state.disk {
            disk.seek_track(absolute_phase)?;
        }
        Ok(())
    }

    fn advance_read_state(&self, state: &mut State) {
        // E7 protection skips some nibbles with write protect sense
        //
        // 00000b59: bd 8c c0        lda c08c,X
        // 00000b5c: 10 fb           bpl b59
        // 00000b5e: c9 e7           cmp #$e7
        // 00000b60: d0 2e           bne b90
        // 00000b62: bd 8d c0        lda c08d,X
        // 00000b65: a0 10           ldy #$10
        // 00000b67: 24 06           bit 06
        // 00000b69: bd 8c c0        lda c08c,X
        // 00000b6c: 10 fb           bpl b69
        // 00000b6e: 88              dey
        // 00000b6f: f0 1f           beq b90
        // 00000b71: c9 ee           cmp #$ee
        //
        // SENSE WRITE PROTECT OR PREWRITE STATE
        if state.q6 && !state.q7 {
            let mut before = state.last_instant;
            let now = self.clock.elapsed();
            while now.duration_since(before) >= Duration::from_micros(4) {
                let _ = state
                    .disk
                    .as_mut()
                    .map(|d| d.read())
                    .unwrap_or(Ok(0))
                    .unwrap_or(0);
                before = before.add_duration(Duration::from_micros(4));
            }
            state.data_register = 0;
            state.last_instant = before;
        }
    }
}

impl Peripheral for DiskIIPeripheral {
    fn read_expansion_rom(&self, address: usize) -> Result<Option<u8>, Error> {
        debug!("read expansion rom {:x?}", address);
        Ok(None)
    }

    fn read_rom(&self, address: usize) -> Result<u8, Error> {
        self.rom.read(address)
    }

    fn device_read(&self, address: usize) -> Result<u8, Error> {
        if address < 8 {
            let phase = (address & !1) >> 1;
            let on = address & 1 == 1;
            self.update_phase(phase, on)?;
            return Ok(0);
        }

        let mut ret = 0;
        match address {
            REGISTER_MOTOR_ON => {
                let mut state = self.state.lock().unwrap();
                state.current_phase.iter_mut().for_each(|p| *p = false);
                trace!("Start Motor Spin");

                // Transition to Motor ON
                if !state.motor_spinning {
                    state.last_instant = self.clock.elapsed();
                }

                state.motor_spinning = true;
            }
            REGISTER_MOTOR_OFF => {
                let mut state = self.state.lock().unwrap();
                state.current_phase.iter_mut().for_each(|p| *p = false);
                trace!("Stop Motor Spin");
                state.motor_off_instant = self.clock.elapsed();
                state.motor_spinning = false;
            }
            REGISTER_SELECT_DRIVE_1 => trace!("Select Drive 1"),
            REGISTER_SELECT_DRIVE_2 => trace!("Select Drive 2"),
            REGISTER_Q7H => {
                let mut state = self.state.lock().unwrap();
                state.q7 = true;
            }
            REGISTER_Q7L => {
                let mut state = self.state.lock().unwrap();
                state.q7 = false;

                // Write Protect Sense Mode / Read Status Register
                if state.q6 && !state.q7 {
                    trace!("read status register");
                    // write protect off
                    //return Ok(1 << 7);
                    ret = state.status.bytes[0];
                }
            }
            REGISTER_Q6H => {
                let mut state = self.state.lock().unwrap();
                state.q6 = true;
            }
            REGISTER_Q6L => {
                let mut state = self.state.lock().unwrap();

                let switch_read_mode = state.q6 && !state.q7;
                state.q6 = false;

                // write mode
                if !state.q6 && state.q7 {
                    // We don't enforce this, we will write each bit every 4 us though
                    //
                    // The execution time of the instructions between the end of two consecutive
                    // parallel load instructions [STA] has to be exactly 32 clock cycles, otherwise
                    // invalid data will be written on the diskette.
                    trace!("write data register {:x}", state.data_register);

                    // if greater than 32us, fill zeros
                    let now = self.clock.elapsed();

                    let mut current = state.last_instant;

                    let bit_duration = Duration::from_secs_f32(0.00000391);
                    let byte_duration = bit_duration.mul_f32(8.0);
                    let mut elapsed = now.duration_since(current);

                    // TODO
                    // right now seems like sometimes we fill an extra bit and cycle count is 36, need to review,
                    // so skew with 1.5
                    if elapsed > byte_duration
                        && (elapsed - byte_duration) > bit_duration.mul_f32(1.5)
                    {
                        let zeros =
                            ((elapsed - byte_duration).div_duration_f32(bit_duration)) as usize;

                        /*debug!(
                            "fill {}, diff {:?}, s: {}, e: {}, d: {}",
                            zeros,
                            now.duration_since(current),
                            current.instant,
                            now.instant,
                            now.instant - current.instant
                        );*/
                        for _ in 0..zeros {
                            if let Some(disk) = &mut state.disk {
                                disk.write(0)?;
                            }
                            current = current.add_duration(Duration::from_micros(4));
                            elapsed = elapsed - bit_duration;
                        }
                    }

                    while elapsed > bit_duration {
                        let bit = (state.data_register >> 7) & 1;
                        if let Some(disk) = &mut state.disk {
                            disk.write(bit)?;
                        }
                        state.data_register <<= 1;
                        elapsed -= bit_duration;
                    }

                    state.last_instant = now.sub_duration(elapsed);
                }
                // read mode
                else if !state.q6 && !state.q7 {
                    let now = self.clock.elapsed();

                    if switch_read_mode {
                        self.advance_read_state(&mut state);
                    }

                    // allow up to 1 second after motor off for reading
                    if !state.motor_spinning
                        && now.as_duration() > state.motor_off_instant.as_duration()
                        && ((now.as_duration() - state.motor_off_instant.as_duration())
                            > Duration::from_secs(1))
                    {
                        // no op
                    } else {
                        let mut before = state.last_instant;
                        //let mut current_byte = state.current_byte;

                        /*debug!(
                            "{} last cycles, now cycles {}, diff {}",
                            initial.instant,
                            now.instant,
                            now.instant - initial.instant
                        );*/
                        // TODO - instead of looping let's just pass the # of bits to advance and then fetch once
                        loop {
                            // wrapped around
                            if before.as_duration() > now.as_duration() {
                                before = now;
                                break;
                            }

                            if now.duration_since(before) < Duration::from_micros(4) {
                                break;
                            }

                            before = before.add_duration(Duration::from_micros(4));
                            let b = state
                                .disk
                                .as_ref()
                                .map(|d| d.read())
                                .transpose()?
                                .unwrap_or(0);
                            state.shift_register <<= 1;
                            state.shift_register |= b & 1;
                            if state.shift_register >> 7 == 1 {
                                state.data_register = state.shift_register;
                                state.shift_register = 0;
                                state.data_valid_until =
                                    before.clone().add_duration(Duration::from_micros(7));
                            }

                            // no guarantee at this point as to where on the disk we are
                            // TODO - need this?
                            if (now.as_duration() > before.as_duration())
                                && (now.as_duration() - before.as_duration()
                                    > Duration::from_micros(200))
                            {
                                //before = now;
                                //break;
                            }
                        }
                        state.last_instant = before;

                        if state.data_valid_until.as_duration() < before.as_duration() {
                            state.data_register = state.shift_register;
                        }
                        let result = state.data_register;

                        // clear data register on read if MSB set
                        if result >> 7 == 1 {
                            state.data_register = 0;
                        }
                        ret = result;
                    }
                }
            }
            _ => {
                debug!("unhandled disk register: 0x{:02x}", address);
            }
        }

        self.advance_read_state(&mut self.state.lock().unwrap());

        Ok(ret)
    }

    fn device_write(&self, address: usize, value: u8) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap();
        trace!("write reg {:x} -> {:x}", address, value);

        state.data_register = value;
        if address == REGISTER_Q6H || address == REGISTER_Q7H {
            trace!("latch data -> {:x}", value);
        }

        if address == REGISTER_Q6H {
            state.q6 = true;
        } else if address == REGISTER_Q6L {
            state.q6 = false;
        } else if address == REGISTER_Q7H {
            //if !state.q7 {
            state.last_instant = self.clock.elapsed();
            // }
            state.q7 = true;
        } else if address == REGISTER_Q7L {
            state.q7 = false;
        }

        // IWM
        if state.q7 && state.q6 && !state.motor_spinning {
            trace!("write mode register -> {:08b}", value);
            let mode = Mode::from_bytes([value]);
            state.status.set_latch(mode.latch());
            state.status.set_asynchronous(mode.asynchronous());
            state.status.set_disable_timer(mode.disable_timer());
            state.status.set_fast_mode(mode.fast_mode());
            state.status.set_clock_8_mhz(mode.clock_8_mhz());

            if mode.mz_reset() {
                state.status.set_mz(false);
            }
        }

        self.advance_read_state(&mut state);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "Disk II"
    }

    fn reset(&self) {
        DiskIIPeripheral::reset(&self);
    }
}
