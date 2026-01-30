use std::sync::Arc;

use crate::{
    clock::ClockTickListener,
    cpu::mos6502::{
        bus::Bus,
        opcodes::{self, Opcode},
    },
    targets::ExecutionState,
    utils::time::Instant,
};

#[derive(Debug, Default, Clone, Copy)]
pub struct Status {
    pub carry: bool,
    pub zero: bool,
    pub irq_disable: bool,
    pub decimal_mode: bool,
    pub brk_command: bool,
    pub overflow: bool,
    pub negative: bool,
}

impl From<u8> for Status {
    fn from(value: u8) -> Self {
        Status {
            carry: (value & 1) != 0,
            zero: ((value >> 1) & 1) != 0,
            irq_disable: ((value >> 2) & 1) != 0,
            decimal_mode: ((value >> 3) & 1) != 0,
            brk_command: false,
            overflow: ((value >> 6) & 1) != 0,
            negative: ((value >> 7) & 1) != 0,
        }
    }
}

impl Into<u8> for Status {
    fn into(self) -> u8 {
        let mut status = 1 << 5;
        if self.negative {
            status |= 1 << 7;
        }
        if self.overflow {
            status |= 1 << 6;
        }
        if self.decimal_mode {
            status |= 1 << 3;
        }
        if self.irq_disable {
            status |= 1 << 2;
        }
        if self.zero {
            status |= 1 << 1;
        }
        if self.carry {
            status |= 1;
        }
        status
    }
}

#[derive(Debug, Default)]
pub struct Registers {
    pub pc: u16,
    pub accumulator: u8,
    pub index_y: u8,
    pub index_x: u8,
    pub sp: u8,
    pub status: Status,
}

#[derive(Debug)]
pub enum Operand {
    Immediate(u16),
    Address(u16),
    Implied,
    Accumulator,
    ZeroPageRel(u8, u16),
}

pub struct State {
    pub registers: Registers,
    pub bus: Arc<Bus>,
    pub execution_state: ExecutionState,
    pub tick_listeners: Vec<Box<dyn ClockTickListener>>,
    pub breakpoints: Vec<u16>,
    pub memory_breakpoints: Vec<u16>,
    pub last_cycle: Instant,
    pub last_execution_time: f64,
    pub stack_frame: Vec<u16>,
    pub clock_execution_speed: f32,
}

impl State {
    pub fn next_instruction(&mut self) -> u8 {
        let i = self.bus.read(self.registers.pc);
        self.registers.pc += 1;
        i
    }

    pub fn pc_read_u8(&mut self) -> u8 {
        let val = self.bus.read(self.registers.pc);
        self.registers.pc += 1;
        val
    }

    pub fn pc_read_u16(&mut self) -> u16 {
        let val = self.bus.read_u16(self.registers.pc);
        self.registers.pc += 2;
        val
    }

    pub fn read_arg(&mut self, opcode: &Opcode) -> (Operand, usize) {
        let mut extra_cycles = 0;
        let arg = match opcode.mode {
            opcodes::AddressingMode::Immediate => Operand::Immediate(self.pc_read_u8() as u16),
            opcodes::AddressingMode::Absolute => Operand::Address(self.pc_read_u16()),
            opcodes::AddressingMode::ZeroPage => Operand::Address(self.pc_read_u8() as u16),
            opcodes::AddressingMode::ZeroPageIndirect => {
                let addr = self.pc_read_u8() as u16;
                Operand::Address(self.bus.read_u16(addr))
            }
            opcodes::AddressingMode::Accum => Operand::Accumulator,
            opcodes::AddressingMode::Implied => Operand::Implied,
            opcodes::AddressingMode::IndexX => {
                let addr = self.registers.index_x.wrapping_add(self.pc_read_u8());
                Operand::Address(self.bus.read_u16(addr as u16))
            }
            opcodes::AddressingMode::IndexY => {
                let zpl = self.pc_read_u8() as u16;
                let addr = self.bus.read_u16(zpl) as u16;

                let result = addr + self.registers.index_y as u16;
                if addr >> 8 != result >> 8 {
                    extra_cycles = 1;
                }

                Operand::Address(result)
            }
            opcodes::AddressingMode::ZeroPageX => {
                Operand::Address((self.registers.index_x.wrapping_add(self.pc_read_u8())) as u16)
            }
            opcodes::AddressingMode::ZeroPageY => {
                Operand::Address((self.registers.index_y.wrapping_add(self.pc_read_u8())) as u16)
            }
            opcodes::AddressingMode::AbsX => {
                let hi = self.pc_read_u16();

                let result = (self.registers.index_x as u16).wrapping_add(hi);
                if result >> 8 != hi >> 8 {
                    extra_cycles = 1;
                }
                Operand::Address(result)
            }
            opcodes::AddressingMode::AbsY => {
                let hi = self.pc_read_u16();
                let result = (self.registers.index_y as u16).wrapping_add(hi);
                if result >> 8 != hi >> 8 {
                    extra_cycles = 1;
                }
                Operand::Address(result)
            }
            opcodes::AddressingMode::Relative => {
                let operand = self.pc_read_u8();
                let result = ((self.registers.pc as i32).wrapping_add(operand as i8 as i32)) as u16;
                Operand::Address(result)
            }
            opcodes::AddressingMode::Indirect => {
                let addr = self.pc_read_u16();
                Operand::Address(self.bus.read_u16(addr))
            }
            opcodes::AddressingMode::ZeroPageRel => {
                let zp = self.pc_read_u8();
                let val = self.bus.read(zp as u16);
                let result = ((self.registers.pc.wrapping_add(1) as i32)
                    .wrapping_add(self.pc_read_u8() as i8 as i32))
                    as u16;
                Operand::ZeroPageRel(val, result)
            }
            opcodes::AddressingMode::Nop2 => {
                // burn a byte
                let _ = self.pc_read_u8();
                Operand::Implied
            }
            opcodes::AddressingMode::Nop3 => {
                // burn 2 bytes
                let _ = self.pc_read_u16();
                Operand::Implied
            }
            opcodes::AddressingMode::AbsXIndirect => {
                let addr = self.pc_read_u16();
                Operand::Address(
                    self.bus
                        .read_u16(addr.wrapping_add(self.registers.index_x as u16)),
                )
            }
        };
        (arg, extra_cycles)
    }

    pub fn push_sp(&mut self, value: u8) {
        self.bus.write(0x100 + self.registers.sp as u16, value);
        self.registers.sp = self.registers.sp.wrapping_sub(1);
    }

    pub fn pop_sp(&mut self) -> u8 {
        self.registers.sp = self.registers.sp.wrapping_add(1);
        let val = self.bus.read(0x100 + self.registers.sp as u16);
        val
    }

    pub fn push_sp_u16(&mut self, value: u16) {
        self.push_sp((value >> 8) as u8);
        self.push_sp(value as u8);
    }

    pub fn pop_sp_u16(&mut self) -> u16 {
        let lo = self.pop_sp();
        ((self.pop_sp() as u16) << 8) | lo as u16
    }
}
