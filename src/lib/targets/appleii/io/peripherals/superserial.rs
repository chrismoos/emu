use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};

use log::{error, trace};
use modular_bitfield::{bitfield, prelude::*};
use serialport::SerialPort;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    select,
    sync::mpsc::UnboundedReceiver,
};

use crate::{
    cpu::{
        InterruptSource, InterruptTarget,
        mos6502::bus::{Slave, memory::MemoryBank},
    },
    errors::Error,
    peripherals::serial::{SerialDevice, SerialDeviceOptions, SerialParity},
    targets::appleii::io::peripheral::Peripheral,
    utils::{futures::spawn, time::sleep},
};

const REGISTER_DIPSW1: usize = 1;
const REGISTER_DIPSW2: usize = 2;
const REGISTER_DREG: usize = 8;
const REGISTER_STATUS: usize = 9;
const REGISTER_COMMAND: usize = 0x0a;
const REGISTER_CONTROL: usize = 0x0b;

#[bitfield]
struct DipSwitch1 {
    sw6: bool,
    sw5: bool,
    sw4: bool,
    sw3: bool,
    sw2: bool,
    sw1: bool,
    pad: B2,
}

impl Default for DipSwitch1 {
    fn default() -> Self {
        Self::from_bytes([0])
    }
}

#[bitfield]
struct DipSwitch2 {
    clear_to_send: bool,
    sw5: bool,
    sw4: bool,
    sw3: bool,
    pad: bool,
    sw2: bool,
    pad1: bool,
    sw1: bool,
}

impl Default for DipSwitch2 {
    fn default() -> Self {
        Self::from_bytes([0])
    }
}

#[bitfield]
struct StatusRegister {
    parity_error: bool,
    framing_error: bool,
    overrun: bool,
    rx_full: bool,
    tx_empty: bool,
    no_data_carrier: bool,
    no_data_ready: bool,
    irq: bool,
}

impl Default for StatusRegister {
    fn default() -> Self {
        let mut val = Self::from_bytes([0]);
        val.set_tx_empty(true);
        val.set_no_data_carrier(true);
        val.set_no_data_ready(true);
        val
    }
}

#[derive(Debug)]
enum DataBitsConfig {
    Bits8 = 0,
    Bits7 = 1,
    Bits6 = 2,
    Bits5 = 3,
}

#[derive(Debug)]
enum StopBitsConfig {}

#[bitfield]
struct ControlRegister {
    baud_rate: B4,
    use_baud_generator: bool,
    num_data_bits: B2,
    stop_bits: bool,
}

impl Default for ControlRegister {
    fn default() -> Self {
        let mut val = ControlRegister::from_bytes([0]);
        val.set_use_baud_generator(true);
        val.set_num_data_bits(DataBitsConfig::Bits8 as u8);
        val
    }
}

#[derive(Debug)]
enum ParityConfig {
    None = 0,
    Odd = 1,
    Even = 3,
    Mark = 5,
    Space = 7,
}

#[bitfield]
struct CommandRegister {
    dtr_enable: bool,
    irq_enable: bool,
    rts_level: B2,
    echo_mode: bool,
    parity: B3,
}

impl Default for CommandRegister {
    fn default() -> Self {
        let mut val = CommandRegister::from_bytes([0]);
        val.set_parity(ParityConfig::None as u8);
        val
    }
}

#[derive(Default)]
struct State {
    serial_port: Option<Box<dyn SerialPort>>,
    rx_buf: VecDeque<u8>,
    tx_register: Option<u8>,
    dipswitch_1: DipSwitch1,
    dipswitch_2: DipSwitch2,
    status: StatusRegister,
    command: CommandRegister,
    control: ControlRegister,

    serial_tx: Option<tokio::sync::mpsc::UnboundedSender<Option<u8>>>,
}

impl State {
    fn get_parity(&self) -> SerialParity {
        match self.command.parity() {
            1 => SerialParity::Odd,
            3 => SerialParity::Even,
            5 => SerialParity::Mark,
            7 => SerialParity::Space,
            _ => SerialParity::None,
        }
    }

    fn get_baud_rate(&self) -> usize {
        match self.control.baud_rate() {
            1 => 50,
            2 => 75,
            3 => 110,
            4 => 135,
            5 => 150,
            6 => 300,
            7 => 600,
            8 => 1200,
            9 => 1800,
            10 => 2400,
            11 => 3600,
            12 => 4800,
            13 => 7200,
            14 => 9600,
            15 => 19200,
            _ => 9600,
        }
    }
}

const IRQ_SOURCE: InterruptSource = 0xff;

// TODO - update how serial device trait works,
// we should just open the port and if baud/parity/etc,. settings changed we change them directly,
//  TODO - implement scratchpad error statuses
pub struct SuperSerialCard {
    rom: MemoryBank,
    expansion_rom: MemoryBank,
    serial_device: Arc<dyn SerialDevice>,
    state: Arc<Mutex<State>>,
    irq_target: Arc<dyn InterruptTarget + Send + Sync>,
}

impl SuperSerialCard {
    pub fn new(
        rom: &[u8],
        serial_device: Arc<dyn SerialDevice>,
        irq_target: Arc<dyn InterruptTarget + Send + Sync>,
    ) -> SuperSerialCard {
        SuperSerialCard {
            // see Super Serial Card manual for how the ROM is split up
            rom: MemoryBank::new_with_data(&rom[0x700..0x800], true),
            expansion_rom: MemoryBank::new_with_data(&rom[0..0x700], true),
            serial_device,
            irq_target,
            state: Arc::new(Mutex::new(State::default())),
        }
    }

    fn open_serial_port(
        &self,
        mut rx: UnboundedReceiver<Option<u8>>,
        baud: usize,
        parity: SerialParity,
    ) -> Result<(), Error> {
        let state = self.state.clone();
        let irq_target = self.irq_target.clone();
        let serial_device = self.serial_device.clone();
        spawn(async move {
            let mut device = match serial_device
                .open(SerialDeviceOptions {
                    parity,
                    baud,
                })
                .await
            {
                Ok(device) => device,
                Err(e) => {
                    error!("failed to open serial port: {:?}", e);
                    let mut state = state.lock().unwrap();
                    Self::close_serial_port(&mut state, irq_target.as_ref());
                    return;
                }
            };

            state.lock().unwrap().status.set_no_data_carrier(false);
            state.lock().unwrap().status.set_no_data_ready(false);

            loop {
                select! {
                    byte = (rx.recv()) => {
                        if let Some(byte) = byte {
                            if let Some(byte) = byte {
                                if let Err(e) = device.write_all(&[byte]).await {
                                    error!("failed to write to serial port: {:?}", e);
                                    let mut state = state.lock().unwrap();
                                    Self::close_serial_port(&mut state,  irq_target.as_ref());
                                    return;
                                }
                            }
                            else {
                                trace!("Serial port close requested.");
                                    return;
                            }
                        }
                    }
                    byte = (device.read_u8()) => {
                        match byte {
                            Ok(byte) => {
                                {
                                    let mut state = state.lock().unwrap();
                                    state.rx_buf.push_back(byte);
                                    state.status.set_irq(true);
                                    irq_target.trigger_irq(false, IRQ_SOURCE);
                                }

                                // if we push too fast proterm can't handle?
                                sleep(Duration::from_micros(1)).await;
                            },
                            Err(e) => {
                                error!("failed to read from serial port: {:?}", e);
                                let mut state = state.lock().unwrap();
                                Self::close_serial_port(&mut state, irq_target.as_ref());
                                return;
                            }
                        }
                    }
                }
            }
        });
        Ok(())
    }

    fn close_serial_port(state: &mut State, irq_target: &dyn InterruptTarget) {
        trace!("serial port closing");
        state.status.set_irq(false);
        state.status.set_no_data_carrier(true);
        state.status.set_no_data_ready(true);
        irq_target.release_irq(false, IRQ_SOURCE);
    }
}

impl Peripheral for SuperSerialCard {
    fn read_expansion_rom(&self, address: usize) -> Result<Option<u8>, crate::errors::Error> {
        Ok(Some(self.expansion_rom.read(address)?))
    }

    fn read_rom(&self, address: usize) -> Result<u8, crate::errors::Error> {
        Ok(self.rom.read(address)?)
    }

    fn device_read(&self, address: usize) -> Result<u8, crate::errors::Error> {
        let mut state = self.state.lock().unwrap();

        let full = state.rx_buf.iter().len() > 0;
        state.status.set_rx_full(full);

        match address {
            REGISTER_DIPSW1 => Ok(state.dipswitch_1.bytes[0]),
            REGISTER_DIPSW2 => Ok(state.dipswitch_2.bytes[0]),
            REGISTER_DREG => {
                let ret = state.rx_buf.pop_front().unwrap_or(0);
                if state.rx_buf.len() == 0 && state.status.irq() {
                    self.irq_target.release_irq(false, IRQ_SOURCE);
                    state.status.set_irq(false);
                }
                Ok(ret)
            }
            REGISTER_STATUS => Ok(state.status.bytes[0]),
            REGISTER_CONTROL => Ok(state.control.bytes[0]),
            REGISTER_COMMAND => Ok(state.command.bytes[0]),
            _ => Ok(0),
        }
    }

    fn device_write(&self, address: usize, value: u8) -> Result<(), crate::errors::Error> {
        let mut state = self.state.lock().unwrap();
        trace!("super serial write: {:x?} -> {:x?}", address, value);

        let dtr_enabled = state.command.dtr_enable();
        match address {
            REGISTER_DIPSW1 => state.dipswitch_1 = DipSwitch1::from_bytes([value]),
            REGISTER_DIPSW2 => state.dipswitch_2 = DipSwitch2::from_bytes([value]),
            REGISTER_DREG => {
                if state.command.dtr_enable() {
                    if let Some(tx) = state.serial_tx.as_mut() {
                        let _ = tx.send(Some(value & !(1 << 7)));
                    }
                    if state.command.echo_mode() {
                        state.rx_buf.push_back(value);
                    }
                    state.status.set_tx_empty(true);
                }
            }
            REGISTER_STATUS => {}
            REGISTER_CONTROL => state.control = ControlRegister::from_bytes([value]),
            REGISTER_COMMAND => state.command = CommandRegister::from_bytes([value]),
            _ => {}
        }

        if state.command.dtr_enable() && !dtr_enabled {
            trace!("DTR enable");
            let (serial_tx, serial_rx) = tokio::sync::mpsc::unbounded_channel();
            let _ = state.serial_tx.take();
            if let Err(e) =
                self.open_serial_port(serial_rx, state.get_baud_rate(), state.get_parity())
            {
                error!("failed to open serial port: {:?}", e);
            }
            state.serial_tx.replace(serial_tx);
        }

        if !state.command.dtr_enable() && dtr_enabled {
            trace!("DTR disable");
            if let Some(tx) = state.serial_tx.take() {
                let _ = tx.send(None);
            }
            Self::close_serial_port(&mut state, self.irq_target.as_ref());
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "Super Serial Card"
    }

    fn reset(&self) {
        let mut state = self.state.lock().unwrap();
        *state = State::default();
    }
}
