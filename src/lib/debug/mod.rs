use std::{fmt::Display, pin::Pin};

use crate::{errors::Error, targets::ExecutionState};

pub trait Debuggable {
    fn run<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>>;

    // Request a halt. This method does not block and wait for it to be halted, use
    // get_state() to poll.
    fn halt(&self) -> Result<(), Error>;
    fn get_state(&self) -> Result<State, Error>;
    fn step(&self) -> Result<(), Error>;
    fn add_memory_breakpoint(&self, address: usize) -> Result<(), Error>;
    fn list_memory_breakpoints(&self) -> Result<Vec<usize>, Error>;
    fn delete_memory_breakpoint(&self, address: usize) -> Result<(), Error>;
    fn add_breakpoint(&self, address: usize) -> Result<(), Error>;
    fn list_breakpoints(&self) -> Result<Vec<usize>, Error>;
    fn delete_breakpoints(&self, address: usize) -> Result<(), Error>;
    fn get_instructions(&self, address: usize, num: usize) -> Result<Vec<InstructionInfo>, Error>;
    fn read_memory(&self, address: usize, num: usize) -> Result<Vec<u8>, Error>;
    fn backtrace(&self) -> Result<Vec<usize>, Error>;
}

#[derive(Debug)]
pub struct InstructionInfo {
    pub address: usize,
    pub instruction: String,
    pub opcode: Vec<u8>,
}

impl Display for InstructionInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!(
            "{:08x}: {:<16}{}",
            self.address,
            self.opcode
                .iter()
                .map(|s| format!("{:02x}", s))
                .collect::<Vec::<_>>()
                .join(" "),
            self.instruction
        ))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum RegisterSize {
    Size1,
    Size8,
    Size16,
    Size32,
    Size64,
}

#[derive(Debug)]
pub struct Register {
    pub name: String,
    pub value: usize,
    pub size: RegisterSize,
}

impl Display for Register {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("{}: ", &self.name))?;
        let args = match self.size {
            RegisterSize::Size1 => {
                if self.value == 0 {
                    format_args!("0")
                } else {
                    format_args!("1")
                }
            }
            RegisterSize::Size8 => format_args!("0x{:01x}", self.value),
            RegisterSize::Size16 => format_args!("0x{:02x}", self.value),
            RegisterSize::Size32 => format_args!("0x{:04x}", self.value),
            RegisterSize::Size64 => format_args!("0x{:08x}", self.value),
        };
        f.write_fmt(args)
    }
}

#[derive(Debug)]
pub struct State {
    pub registers: Vec<Register>,
    pub flags: Vec<(String, bool)>,
    pub pc: Register,
    pub execution_state: ExecutionState,
}
