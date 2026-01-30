use std::{
    pin::Pin,
    sync::{Arc, Mutex, atomic::AtomicU64},
    time::Duration,
};

use log::{debug, info, trace};

use crate::{
    clock::{Clock, ClockInstant},
    cpu::{
        InterruptSource, InterruptTarget,
        mos6502::{
            bus::Bus,
            opcodes::decode_instruction,
            state::{Operand, State, Status},
        },
    },
    debug::{self, Debuggable, InstructionInfo},
    errors::Error,
    targets::ExecutionState,
    utils::time::{Instant, sleep},
};

pub mod bus;
pub mod opcodes;
pub mod state;

const VECTOR_RESET: u16 = 0xfffc;
const VECTOR_NMI: u16 = 0xfffa;
const VECTOR_IRQ: u16 = 0xfffe;

const OPERAND_UNSUPPORTED: &'static str = "operand unsupported";

pub struct Mos6502 {
    state: Mutex<State>,
    clock_cycle_duration: Duration,
    variant: Variant,
    cycle_count: AtomicU64,
    irq_state: Mutex<IrqState>,
    execution_request: Mutex<ExecutionRequest>,
}

#[derive(Default)]
struct IrqState {
    irq: Vec<InterruptSource>,
    nmi: Vec<InterruptSource>,
    nmi_previous_cycle: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionRequest {
    None,
    Halt,
    Resume,
    ForceBreak,
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    Original,
    Wdc65c02,
}

#[derive(Debug, Clone, Copy)]
pub struct ClockConfig {
    rate_hz: u64,

    // 1.0 for running at the same rate as cycle_duration
    cycle_execution_speed: f32,
}

impl ClockConfig {
    pub fn new(rate_hz: u64, speed: f32) -> ClockConfig {
        ClockConfig {
            rate_hz,
            cycle_execution_speed: speed,
        }
    }
}

impl Mos6502 {
    pub fn new(bus: Arc<Bus>, clock_config: ClockConfig, variant: Variant) -> Mos6502 {
        let mos = Mos6502 {
            variant,
            clock_cycle_duration: Duration::from_secs_f32(1.0 / (clock_config.rate_hz as f32)),
            state: Mutex::new(State {
                bus,
                tick_listeners: vec![],
                clock_execution_speed: clock_config.cycle_execution_speed,
                last_cycle: Instant::now(),
                last_execution_time: 0.0,
                breakpoints: vec![],
                memory_breakpoints: vec![],
                registers: Default::default(),
                execution_state: ExecutionState::Stopped,
                stack_frame: vec![],
            }),
            execution_request: Mutex::new(ExecutionRequest::None),
            irq_state: Mutex::new(IrqState {
                irq: vec![],
                nmi: vec![],
                nmi_previous_cycle: false,
            }),
            cycle_count: AtomicU64::new(0),
        };
        mos
    }

    pub fn irq_trigger(&self, nmi: bool, source: InterruptSource) {
        let mut state = self.irq_state.lock().unwrap();
        if nmi {
            state.nmi.push(source);
        } else {
            state.irq.push(source);
        }
    }

    pub fn irq_release(&self, nmi: bool, source: InterruptSource) {
        let mut state = self.irq_state.lock().unwrap();
        if nmi {
            state.nmi.retain(|s| *s != source);
        } else {
            state.irq.retain(|s| *s != source);
        }
    }

    pub fn get_actual_freq(&self) -> f64 {
        let t = self.state.lock().unwrap().last_execution_time;
        if t == 0.0 { 0.0 } else { 1.0 / t }
    }

    // There are two major facts to remember about initialization. One, the only automatic operations of the microprocessor during reset are to turn
    // on the interrupt disable bit and to force the program counter to the vector location specified in locations FFFC and FFFD and to load the first
    // instruction from that location.
    pub fn reset(&self) {
        let mut state = self.state.lock().unwrap();

        state.registers = Default::default();
        state.registers.pc = state.bus.read_u16(VECTOR_RESET);
        trace!(
            "PC loaded from reset vector {:x?} -> {:x?}",
            VECTOR_RESET, state.registers.pc
        );
        state.registers.status.irq_disable = true;
        state.stack_frame.clear();
        state.last_cycle = Instant::now();
        state.last_execution_time = 0.0;

        *self.irq_state.lock().unwrap() = Default::default();

        self.set_cycle_count(0);
    }

    fn set_cycle_count(&self, num: u64) {
        self.cycle_count
            .store(num, std::sync::atomic::Ordering::SeqCst);
    }

    pub async fn start(self: Arc<Self>) -> Result<(), Error> {
        self.execute().await
    }

    pub fn stop(&self) -> Result<(), Error> {
        *self.execution_request.lock().unwrap() = ExecutionRequest::Shutdown;
        Ok(())
    }

    pub fn inc_cycles(&self, num: u64) {
        self.cycle_count
            .fetch_add(num, std::sync::atomic::Ordering::SeqCst);
    }

    fn check_nmi(&self) -> bool {
        let mut state = self.state.lock().unwrap();
        let mut irq_state = self.irq_state.lock().unwrap();

        // only triggered if an edge is detected
        let nmi_trigger = irq_state.nmi.len() > 0 && !irq_state.nmi_previous_cycle;

        if nmi_trigger {
            let pc = state.registers.pc;
            let status = state.registers.status.into();
            state.push_sp_u16(pc);
            state.push_sp(status);

            trace!(
                "NMI handle (nmi={}), current pc {:x}, stack {:x?}",
                nmi_trigger, pc, state.stack_frame
            );

            state.registers.pc = state.bus.read_u16(VECTOR_NMI);
            state.registers.status.irq_disable = true;
            state.registers.status.brk_command = false;

            if self.variant == Variant::Wdc65c02 {
                state.registers.status.decimal_mode = false;
            }

            irq_state.nmi_previous_cycle = true;
            true
        } else {
            irq_state.nmi_previous_cycle = false;
            false
        }
    }

    fn check_irqs(&self) {
        let mut state = self.state.lock().unwrap();
        let irq_state = self.irq_state.lock().unwrap();

        let irq_trigger = irq_state.irq.len() > 0 && !state.registers.status.irq_disable;

        if irq_trigger {
            let pc = state.registers.pc;
            let status = state.registers.status.into();
            state.push_sp_u16(pc);
            state.push_sp(status);

            trace!(
                "IRQ handle (irq={}), current pc {:x}, stack {:x?}",
                irq_trigger, pc, state.stack_frame
            );

            state.registers.pc = state.bus.read_u16(VECTOR_IRQ);
            state.registers.status.irq_disable = true;
            state.registers.status.brk_command = false;

            if self.variant == Variant::Wdc65c02 {
                state.registers.status.decimal_mode = false;
            }
        }
    }

    fn execute_single(&self) -> Result<(), Error> {
        if !self.check_nmi() {
            self.check_irqs();
        }

        let mut state = self.state.lock().unwrap();
        let current_pc = state.registers.pc;
        let next_instruction = state.next_instruction();
        match decode_instruction(next_instruction) {
            Some(opcode) => {
                let (arg, extra_cycles) = state.read_arg(&opcode.opcode());

                /*debug!(
                    "pc: {:x}, cycles {}, total {}, opcode: {:?}",
                    current_pc,
                    (opcode.opcode().cycles + extra_cycles) as u64,
                    self.get_cycle_count(),
                    opcode,
                );*/

                self.inc_cycles((opcode.opcode().cycles + extra_cycles) as u64);

                /*debug!(
                    "pc {:x}, {:?}, state {:x?}",
                    current_pc,
                    opcode.opcode(),
                    state.registers
                );*/
                let is_65c02 = self.variant == Variant::Wdc65c02;

                match opcode.opcode().instruction {
                    opcodes::Instruction::Adc => self.instr_adc(arg, &mut state)?,
                    opcodes::Instruction::Jsr => self.instr_jsr(arg, &mut state, current_pc)?,
                    opcodes::Instruction::And => self.instr_and(arg, &mut state)?,
                    opcodes::Instruction::Asl => self.instr_asl(arg, &mut state)?,
                    opcodes::Instruction::Bcc => {
                        if !state.registers.status.carry {
                            self.instr_branch(arg, &mut state)?
                        }
                    }
                    opcodes::Instruction::Bcs => self.instr_bcs(arg, &mut state)?,
                    opcodes::Instruction::Beq => self.instr_beq(arg, &mut state)?,
                    opcodes::Instruction::Bit => self.instr_bit(arg, &mut state)?,
                    opcodes::Instruction::Bmi => self.instr_bmi(arg, &mut state)?,
                    opcodes::Instruction::Bne => self.instr_bne(arg, &mut state)?,
                    opcodes::Instruction::Bpl => self.instr_bpl(arg, &mut state)?,
                    opcodes::Instruction::Brk => self.instr_brk(arg, &mut state)?,
                    opcodes::Instruction::Bvc => self.instr_bvc(arg, &mut state)?,
                    opcodes::Instruction::Bvs => self.instr_bvs(arg, &mut state)?,
                    opcodes::Instruction::Clc => self.instr_clc(arg, &mut state)?,
                    opcodes::Instruction::Cld => self.instr_cld(arg, &mut state)?,
                    opcodes::Instruction::Cli => self.instr_cli(arg, &mut state)?,
                    opcodes::Instruction::Clv => self.instr_clv(arg, &mut state)?,
                    opcodes::Instruction::Cmp => self.instr_cmp(arg, &mut state)?,
                    opcodes::Instruction::Cpx => self.instr_cpx(arg, &mut state)?,
                    opcodes::Instruction::Cpy => self.instr_cpy(arg, &mut state)?,
                    opcodes::Instruction::Dec => self.instr_dec(arg, &mut state)?,
                    opcodes::Instruction::Dex => self.instr_dex(arg, &mut state)?,
                    opcodes::Instruction::Dey => self.instr_dey(arg, &mut state)?,
                    opcodes::Instruction::Eor => self.instr_eor(arg, &mut state)?,
                    opcodes::Instruction::Inc => self.instr_inc(arg, &mut state)?,
                    opcodes::Instruction::Inx => self.instr_inx(arg, &mut state)?,
                    opcodes::Instruction::Iny => self.instr_iny(arg, &mut state)?,
                    opcodes::Instruction::Jmp => self.instr_jmp(arg, &mut state)?,
                    opcodes::Instruction::Lda => self.instr_ld(arg, &mut state, |val, state| {
                        state.registers.accumulator = val
                    })?,
                    opcodes::Instruction::Ldx => {
                        self.instr_ld(arg, &mut state, |val, state| state.registers.index_x = val)?
                    }
                    opcodes::Instruction::Ldy => {
                        self.instr_ld(arg, &mut state, |val, state| state.registers.index_y = val)?
                    }
                    opcodes::Instruction::Lsr => self.instr_lsr(arg, &mut state)?,
                    opcodes::Instruction::Nop => {}
                    opcodes::Instruction::NopC2 => {}
                    opcodes::Instruction::Ora => self.instr_ora(arg, &mut state)?,
                    opcodes::Instruction::Pha => self.instr_pha(arg, &mut state)?,
                    opcodes::Instruction::Php => self.instr_php(arg, &mut state)?,
                    opcodes::Instruction::Pla => self.instr_pla(arg, &mut state)?,
                    opcodes::Instruction::Plp => self.instr_plp(arg, &mut state)?,
                    opcodes::Instruction::Rol => self.instr_rol(arg, &mut state)?,
                    opcodes::Instruction::Ror => self.instr_ror(arg, &mut state)?,
                    opcodes::Instruction::Rti => self.instr_rti(arg, &mut state)?,
                    opcodes::Instruction::Rts => self.instr_rts(arg, &mut state)?,
                    opcodes::Instruction::Sbc => self.instr_sbc(arg, &mut state)?,
                    opcodes::Instruction::Sec => self.instr_sec(arg, &mut state)?,
                    opcodes::Instruction::Sed => self.instr_sed(arg, &mut state)?,
                    opcodes::Instruction::Sei => self.instr_sei(arg, &mut state)?,
                    opcodes::Instruction::Sta => self.instr_sta(arg, &mut state)?,
                    opcodes::Instruction::Stx => self.instr_stx(arg, &mut state)?,
                    opcodes::Instruction::Sty => self.instr_sty(arg, &mut state)?,
                    opcodes::Instruction::Tax => self.instr_tax(arg, &mut state)?,
                    opcodes::Instruction::Tay => self.instr_tay(arg, &mut state)?,
                    opcodes::Instruction::Tsx => self.instr_tsx(arg, &mut state)?,
                    opcodes::Instruction::Txa => self.instr_txa(arg, &mut state)?,
                    opcodes::Instruction::Txs => self.instr_txs(arg, &mut state)?,
                    opcodes::Instruction::Tya => self.instr_tya(arg, &mut state)?,
                    opcodes::Instruction::Bra if is_65c02 => self.instr_bra(arg, &mut state)?,
                    opcodes::Instruction::Bbr0 if is_65c02 => self.instr_bbr(arg, &mut state, 1)?,
                    opcodes::Instruction::Bbr1 if is_65c02 => self.instr_bbr(arg, &mut state, 2)?,
                    opcodes::Instruction::Bbr2 if is_65c02 => self.instr_bbr(arg, &mut state, 4)?,
                    opcodes::Instruction::Bbr3 if is_65c02 => self.instr_bbr(arg, &mut state, 8)?,
                    opcodes::Instruction::Bbr4 if is_65c02 => {
                        self.instr_bbr(arg, &mut state, 16)?
                    }
                    opcodes::Instruction::Bbr5 if is_65c02 => {
                        self.instr_bbr(arg, &mut state, 32)?
                    }
                    opcodes::Instruction::Bbr6 if is_65c02 => {
                        self.instr_bbr(arg, &mut state, 64)?
                    }
                    opcodes::Instruction::Bbr7 if is_65c02 => {
                        self.instr_bbr(arg, &mut state, 128)?
                    }
                    opcodes::Instruction::Bbs0 if is_65c02 => self.instr_bbs(arg, &mut state, 1)?,
                    opcodes::Instruction::Bbs1 if is_65c02 => self.instr_bbs(arg, &mut state, 2)?,
                    opcodes::Instruction::Bbs2 if is_65c02 => self.instr_bbs(arg, &mut state, 4)?,
                    opcodes::Instruction::Bbs3 if is_65c02 => self.instr_bbs(arg, &mut state, 8)?,
                    opcodes::Instruction::Bbs4 if is_65c02 => {
                        self.instr_bbs(arg, &mut state, 16)?
                    }
                    opcodes::Instruction::Bbs5 if is_65c02 => {
                        self.instr_bbs(arg, &mut state, 32)?
                    }
                    opcodes::Instruction::Bbs6 if is_65c02 => {
                        self.instr_bbs(arg, &mut state, 64)?
                    }
                    opcodes::Instruction::Bbs7 if is_65c02 => {
                        self.instr_bbs(arg, &mut state, 128)?
                    }
                    opcodes::Instruction::Phx if is_65c02 => self.instr_phx(arg, &mut state)?,
                    opcodes::Instruction::Phy if is_65c02 => self.instr_phy(arg, &mut state)?,
                    opcodes::Instruction::Plx if is_65c02 => self.instr_plx(arg, &mut state)?,
                    opcodes::Instruction::Ply if is_65c02 => self.instr_ply(arg, &mut state)?,
                    opcodes::Instruction::Rmb0 if is_65c02 => self.instr_rmb(arg, &mut state, 1)?,
                    opcodes::Instruction::Rmb1 if is_65c02 => self.instr_rmb(arg, &mut state, 2)?,
                    opcodes::Instruction::Rmb2 if is_65c02 => self.instr_rmb(arg, &mut state, 4)?,
                    opcodes::Instruction::Rmb3 if is_65c02 => self.instr_rmb(arg, &mut state, 8)?,
                    opcodes::Instruction::Rmb4 if is_65c02 => {
                        self.instr_rmb(arg, &mut state, 16)?
                    }
                    opcodes::Instruction::Rmb5 if is_65c02 => {
                        self.instr_rmb(arg, &mut state, 32)?
                    }
                    opcodes::Instruction::Rmb6 if is_65c02 => {
                        self.instr_rmb(arg, &mut state, 64)?
                    }
                    opcodes::Instruction::Rmb7 if is_65c02 => {
                        self.instr_rmb(arg, &mut state, 128)?
                    }
                    opcodes::Instruction::Smb0 if is_65c02 => self.instr_smb(arg, &mut state, 1)?,
                    opcodes::Instruction::Smb1 if is_65c02 => self.instr_smb(arg, &mut state, 2)?,
                    opcodes::Instruction::Smb2 if is_65c02 => self.instr_smb(arg, &mut state, 4)?,
                    opcodes::Instruction::Smb3 if is_65c02 => self.instr_smb(arg, &mut state, 8)?,
                    opcodes::Instruction::Smb4 if is_65c02 => {
                        self.instr_smb(arg, &mut state, 16)?
                    }
                    opcodes::Instruction::Smb5 if is_65c02 => {
                        self.instr_smb(arg, &mut state, 32)?
                    }
                    opcodes::Instruction::Smb6 if is_65c02 => {
                        self.instr_smb(arg, &mut state, 64)?
                    }
                    opcodes::Instruction::Smb7 if is_65c02 => {
                        self.instr_smb(arg, &mut state, 128)?
                    }
                    opcodes::Instruction::Stp if is_65c02 => self.instr_stp(arg, &mut state)?,
                    opcodes::Instruction::Stz if is_65c02 => self.instr_stz(arg, &mut state)?,
                    opcodes::Instruction::Trb if is_65c02 => self.instr_trb(arg, &mut state)?,
                    opcodes::Instruction::Tsb if is_65c02 => self.instr_tsb(arg, &mut state)?,
                    opcodes::Instruction::Wai if is_65c02 => self.instr_wai(arg, &mut state)?,
                    opcodes::Instruction::Nop03 if is_65c02 => {}
                    opcodes::Instruction::Nop13 if is_65c02 => {}
                    opcodes::Instruction::Nop23 if is_65c02 => {}
                    opcodes::Instruction::Nop33 if is_65c02 => {}
                    opcodes::Instruction::Nop43 if is_65c02 => {}
                    opcodes::Instruction::Nop53 if is_65c02 => {}
                    opcodes::Instruction::Nop63 if is_65c02 => {}
                    opcodes::Instruction::Nop73 if is_65c02 => {}
                    opcodes::Instruction::Nop83 if is_65c02 => {}
                    opcodes::Instruction::Nop93 if is_65c02 => {}
                    opcodes::Instruction::NopA3 if is_65c02 => {}
                    opcodes::Instruction::NopB3 if is_65c02 => {}
                    opcodes::Instruction::NopC3 if is_65c02 => {}
                    opcodes::Instruction::NopD3 if is_65c02 => {}
                    opcodes::Instruction::NopE3 if is_65c02 => {}
                    opcodes::Instruction::NopF3 if is_65c02 => {}
                    opcodes::Instruction::Nop0B if is_65c02 => {}
                    opcodes::Instruction::Nop1B if is_65c02 => {}
                    opcodes::Instruction::Nop2B if is_65c02 => {}
                    opcodes::Instruction::Nop3B if is_65c02 => {}
                    opcodes::Instruction::Nop4B if is_65c02 => {}
                    opcodes::Instruction::Nop5B if is_65c02 => {}
                    opcodes::Instruction::Nop6B if is_65c02 => {}
                    opcodes::Instruction::Nop7B if is_65c02 => {}
                    opcodes::Instruction::Nop8B if is_65c02 => {}
                    opcodes::Instruction::Nop9B if is_65c02 => {}
                    opcodes::Instruction::NopAB if is_65c02 => {}
                    opcodes::Instruction::NopBB if is_65c02 => {}
                    opcodes::Instruction::NopEB if is_65c02 => {}
                    opcodes::Instruction::NopFB if is_65c02 => {}
                    opcodes::Instruction::Nop02 if is_65c02 => {}
                    opcodes::Instruction::Nop22 if is_65c02 => {}
                    opcodes::Instruction::Nop42 if is_65c02 => {}
                    opcodes::Instruction::Nop62 if is_65c02 => {}
                    opcodes::Instruction::Nop82 if is_65c02 => {}
                    opcodes::Instruction::Nope2 if is_65c02 => {}
                    opcodes::Instruction::Nop44 if is_65c02 => {}
                    opcodes::Instruction::Nop54 if is_65c02 => {}
                    opcodes::Instruction::NopD4 if is_65c02 => {}
                    opcodes::Instruction::Nopf4 if is_65c02 => {}
                    opcodes::Instruction::NopDc if is_65c02 => {}
                    opcodes::Instruction::NopFc if is_65c02 => {}
                    opcodes::Instruction::Nop5c if is_65c02 => {}
                    _ => {
                        info!(
                            "unsupported instruction: 0x{:02x}: {}",
                            next_instruction, opcode
                        );
                    } // nop
                }
            }
            None => {
                return Err(format!("unsupported instruction: 0x{:02x}", next_instruction).into());
            }
        }

        state.tick_listeners.iter_mut().for_each(|l| {
            l.tick_updated(ClockInstant {
                instant: self.get_cycle_count(),
                tick_duration: self.clock_cycle_duration,
            })
        });

        Ok(())
    }

    pub fn force_break(&self) -> Result<(), Error> {
        *self.execution_request.lock().unwrap() = ExecutionRequest::ForceBreak;
        Ok(())
    }

    pub async fn execute(&self) -> Result<(), Error> {
        loop {
            let req = *self.execution_request.lock().unwrap();
            if req == ExecutionRequest::Halt || req == ExecutionRequest::Shutdown {
                debug!("Halt requested, execution stopped.");
                self.state.lock().unwrap().execution_state = ExecutionState::Stopped;
            }

            if self.state.lock().unwrap().execution_state == ExecutionState::Stopped {
                debug!("CPU Stopped.");
                while *self.execution_request.lock().unwrap() != ExecutionRequest::Resume {
                    sleep(Duration::from_millis(10)).await;
                    if *self.execution_request.lock().unwrap() == ExecutionRequest::Shutdown {
                        debug!("Shutdown requested, exiting...");
                        return Ok(());
                    }
                }
                debug!("Resuming execution...");
                *self.execution_request.lock().unwrap() = ExecutionRequest::None;
                self.state.lock().unwrap().execution_state = ExecutionState::Running;
            }

            // run batch of instructions, easier to handle timing as a single cycle
            // is quite fast
            let batch_size = Duration::from_secs_f32(1.0 / 60.0)
                .div_duration_f32(self.clock_cycle_duration) as usize;

            let start_count = self.get_cycle_count();
            let start_time = Instant::now();
            for _ in 0..batch_size {
                let mut bp_hit =
                    *self.execution_request.lock().unwrap() == ExecutionRequest::ForceBreak;
                {
                    let s = self.state.lock().unwrap();
                    for bp in &s.breakpoints {
                        if s.registers.pc == *bp {
                            info!("Breakpoint at 0x{:04x} hit!", bp);
                            bp_hit = true;
                            break;
                        }
                    }
                }

                if bp_hit {
                    debug!("CPU Stopped on breakpoint.");
                    while *self.execution_request.lock().unwrap() != ExecutionRequest::Resume {
                        sleep(Duration::from_millis(10)).await;
                    }
                    debug!("Resuming execution...");
                    *self.execution_request.lock().unwrap() = ExecutionRequest::None;
                    self.state.lock().unwrap().execution_state = ExecutionState::Running;
                }

                self.execute_single()?;
            }
            let end_count = self.get_cycle_count();

            let sleep_duration = self
                .clock_cycle_duration
                .mul_f32((end_count - start_count) as f32)
                .div_f32(self.state.lock().unwrap().clock_execution_speed);

            let until = start_time + sleep_duration;

            // In async mode, give other tasks some time to run by sleeping.
            // We don't sleep the whole duration due to lack of precision of sleep.
            /*debug!(
                "sleep for {:?}, we took {:?} to run",
                sleep_duration.div_f32(2.0),
                Instant::now() - start_time
            );*/
            #[cfg(feature = "wasm")]
            sleep(Duration::from_secs(0)).await;
            #[cfg(not(feature = "wasm"))]
            sleep(sleep_duration.div_f32(4.0)).await;
            //debug!("slept for {:?}", Instant::now() - start1);

            while Instant::now() < until {
                //debug!("instant: {:?}, until {:?}", Instant::now(), until);
            }
            self.state.lock().unwrap().last_execution_time = (Instant::now() - start_time)
                .div_f64((end_count - start_count) as f64)
                .as_secs_f64();
        }
    }

    pub fn set_execution_speed(&self, speed: f32) {
        self.state.lock().unwrap().clock_execution_speed = speed;
    }

    pub fn get_execution_speed(&self) -> f32 {
        self.state.lock().unwrap().clock_execution_speed
    }

    pub fn set_pc(&self, pc: u16) -> Result<(), Error> {
        self.state.lock().unwrap().registers.pc = pc;
        Ok(())
    }

    fn get_cycle_count(&self) -> u64 {
        self.cycle_count.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn instr_bra(&self, arg: Operand, state: &mut State) -> Result<(), Error> {
        self.instr_branch(arg, state)
    }

    fn instr_bbs(&self, arg: Operand, state: &mut State, bit: u8) -> Result<(), Error> {
        match arg {
            Operand::ZeroPageRel(val, branch) => {
                if val & bit == bit {
                    state.registers.pc = branch;
                }
            }
            _ => return Err("invalid addressing mode".into()),
        }
        Ok(())
    }

    fn instr_smb(&self, arg: Operand, state: &mut State, bit: u8) -> Result<(), Error> {
        match arg {
            Operand::Address(addr) => {
                let mut val = state.bus.read(addr);
                val |= bit;
                state.bus.write(addr, val);
            }
            _ => return Err("invalid addressing mode".into()),
        }
        Ok(())
    }

    fn instr_rmb(&self, arg: Operand, state: &mut State, bit: u8) -> Result<(), Error> {
        match arg {
            Operand::Address(addr) => {
                let mut val = state.bus.read(addr);
                val &= !bit;
                state.bus.write(addr, val);
            }
            _ => return Err("invalid addressing mode".into()),
        }
        Ok(())
    }

    fn instr_bbr(&self, arg: Operand, state: &mut State, bit: u8) -> Result<(), Error> {
        match arg {
            Operand::ZeroPageRel(val, branch) => {
                if val & bit == 0 {
                    state.registers.pc = branch;
                }
            }
            _ => return Err("invalid addressing mode".into()),
        }
        Ok(())
    }

    fn instr_ld<F>(
        &self,
        arg: Operand,
        state: &mut State,
        destination_writer: F,
    ) -> Result<(), Error>
    where
        F: FnOnce(u8, &mut State),
    {
        let value = match arg {
            Operand::Immediate(imm) => imm as u8,
            Operand::Address(addr) => state.bus.read(addr),
            _ => return Err(OPERAND_UNSUPPORTED.into()),
        };

        destination_writer(value, state);

        self.update_negative_flag(value, state);
        self.update_zero_flag(value, state);

        Ok(())
    }

    fn update_negative_flag(&self, value: u8, state: &mut State) {
        state.registers.status.negative = value >> 7 == 1;
    }

    fn update_zero_flag(&self, value: u8, state: &mut State) {
        state.registers.status.zero = value == 0;
    }

    fn instr_sbc(&self, arg: Operand, state: &mut State) -> Result<(), Error> {
        if state.registers.status.decimal_mode && self.variant == Variant::Wdc65c02 {
            self.inc_cycles(1);
        }

        let value = match arg {
            Operand::Immediate(val) => val as u8,
            Operand::Address(addr) => state.bus.read(addr),
            _ => return Err(OPERAND_UNSUPPORTED.into()),
        };

        if state.registers.status.decimal_mode {
            self.instr_adc_val(!value, state, true)
        } else {
            self.instr_adc_val(!value, state, true)
        }
    }

    fn instr_adc(&self, arg: Operand, state: &mut State) -> Result<(), Error> {
        if state.registers.status.decimal_mode && self.variant == Variant::Wdc65c02 {
            self.inc_cycles(1);
        }

        let value = match arg {
            Operand::Immediate(val) => val as u8,
            Operand::Address(addr) => state.bus.read(addr),
            _ => return Err(OPERAND_UNSUPPORTED.into()),
        };

        self.instr_adc_val(value, state, false)
    }

    fn nibble_adder(&self, m: u8, n: u8, carry: bool) -> (u8, bool) {
        let result = m + n;
        return (result & 0b1111, result > 15 || carry);
    }

    fn add_nibble_with_carry(
        &self,
        m: u8,
        n: u8,
        carry: bool,
        _correction: i8,
        subtraction: bool,
    ) -> (u8, bool) {
        let mut result = m + n;
        if carry {
            result += 1;
        }

        let carry_out = (result & (1 << 4)) != 0;
        result &= 0b1111;
        if result > 9 || (!subtraction && carry_out) || (subtraction && !carry_out) {
            if subtraction {
                ((result - 6) & 0b1111, carry_out)
            } else {
                self.nibble_adder(result, 6, carry_out)
            }
        } else {
            (result, carry_out)
        }
    }

    fn instr_adc_val_decimal(
        &self,
        value: u8,
        state: &mut State,
        subtraction: bool,
    ) -> Result<(), Error> {
        let m_hi = (state.registers.accumulator >> 4) & 0b1111;
        let m_lo = state.registers.accumulator & 0b1111;

        let n_hi = (value >> 4) & 0b1111;
        let n_lo = value & 0b1111;

        let (lo_result, lo_carry) = self.add_nibble_with_carry(
            m_lo,
            n_lo,
            state.registers.status.carry,
            if subtraction { -6 } else { 6 },
            subtraction,
        );

        let (hi_result, hi_carry) = self.add_nibble_with_carry(
            m_hi,
            n_hi,
            lo_carry,
            if subtraction { -6 } else { 6 },
            subtraction,
        );

        let result = (hi_result << 4) + lo_result;

        /*println!(
            "{:x} + {:x} + {} = {:x}, lo_result = {}, hi_result = {}, sub: {}",
            state.registers.accumulator,
            value,
            state.registers.status.carry,
            result,
            lo_result,
            hi_result,
            subtraction
        );*/
        state.registers.accumulator = result;
        state.registers.status.carry = hi_carry;
        state.registers.status.overflow = false;
        self.update_negative_flag(result, state);
        self.update_zero_flag(result, state);

        Ok(())
    }
    fn instr_adc_val(&self, value: u8, state: &mut State, subtraction: bool) -> Result<(), Error> {
        if state.registers.status.decimal_mode {
            self.instr_adc_val_decimal(value, state, subtraction)
        } else {
            self.instr_adc_val_binary(value, state)
        }
    }

    fn instr_adc_val_binary(&self, value: u8, state: &mut State) -> Result<(), Error> {
        let (mut result, mut carry) = state.registers.accumulator.overflowing_add(value);
        if state.registers.status.carry {
            let (carry_result, carry2) = result.overflowing_add(1);
            if carry2 {
                carry = true;
            }
            result = carry_result;
        }

        state.registers.status.carry = carry;
        self.update_negative_flag(result, state);
        self.update_zero_flag(result, state);

        if ((state.registers.accumulator >> 7) & 1) != 0
            && ((value >> 7) & 1) != 0
            && ((result >> 7) & 1) == 0
        {
            state.registers.status.overflow = true;
        } else if ((state.registers.accumulator >> 7) & 1) == 0
            && ((value >> 7) & 1) == 0
            && ((result >> 7) & 1) != 0
        {
            state.registers.status.overflow = true;
        } else {
            state.registers.status.overflow = false;
        }

        state.registers.accumulator = result;

        Ok(())
    }

    fn get_operand_addr(&self, arg: &Operand, _state: &mut State) -> Result<u16, Error> {
        match arg {
            Operand::Address(addr) => Ok(*addr),
            _ => return Err(OPERAND_UNSUPPORTED.into()),
        }
    }

    fn get_operand_addr_imm(&self, arg: &Operand, state: &mut State) -> Result<u8, Error> {
        match arg {
            Operand::Address(addr) => Ok(state.bus.read(*addr)),
            Operand::Immediate(val) => Ok(*val as u8),
            _ => return Err(OPERAND_UNSUPPORTED.into()),
        }
    }

    fn instr_clc(&self, _arg: Operand, state: &mut State) -> Result<(), Error> {
        state.registers.status.carry = false;
        Ok(())
    }

    fn instr_clv(&self, _arg: Operand, state: &mut State) -> Result<(), Error> {
        state.registers.status.overflow = false;
        Ok(())
    }

    fn instr_cli(&self, _arg: Operand, state: &mut State) -> Result<(), Error> {
        state.registers.status.irq_disable = false;
        Ok(())
    }

    fn instr_cld(&self, _arg: Operand, state: &mut State) -> Result<(), Error> {
        state.registers.status.decimal_mode = false;
        Ok(())
    }

    fn instr_bvs(&self, arg: Operand, state: &mut State) -> Result<(), Error> {
        if state.registers.status.overflow {
            self.instr_branch(arg, state)?;
        }
        Ok(())
    }

    fn instr_bvc(&self, arg: Operand, state: &mut State) -> Result<(), Error> {
        if !state.registers.status.overflow {
            self.instr_branch(arg, state)?;
        }
        Ok(())
    }

    fn instr_bpl(&self, arg: Operand, state: &mut State) -> Result<(), Error> {
        if !state.registers.status.negative {
            self.instr_branch(arg, state)?;
        }
        Ok(())
    }

    fn instr_brk(&self, _arg: Operand, state: &mut State) -> Result<(), Error> {
        state.push_sp_u16(state.registers.pc + 1);

        let status: u8 = state.registers.status.into();
        state.push_sp(status | (1 << 4)); // Set break flag
        state.registers.pc = state.bus.read_u16(VECTOR_IRQ);
        state.registers.status.irq_disable = true;
        if self.variant == Variant::Wdc65c02 {
            state.registers.status.decimal_mode = false;
        }

        Ok(())
    }

    fn instr_bne(&self, arg: Operand, state: &mut State) -> Result<(), Error> {
        if !state.registers.status.zero {
            self.instr_branch(arg, state)?;
        }
        Ok(())
    }

    fn instr_bmi(&self, arg: Operand, state: &mut State) -> Result<(), Error> {
        if state.registers.status.negative {
            self.instr_branch(arg, state)?;
        }
        Ok(())
    }

    fn instr_wai(&self, _arg: Operand, _state: &mut State) -> Result<(), Error> {
        // TODO - interrupts
        // wai not yet supported
        Err("wai: not supported".into())
    }

    fn instr_tsb(&self, arg: Operand, state: &mut State) -> Result<(), Error> {
        match arg {
            Operand::Address(addr) => {
                let mut val = state.bus.read(addr);
                self.update_zero_flag(val & state.registers.accumulator, state);
                val |= state.registers.accumulator;
                state.bus.write(addr, val);
            }
            _ => return Err(OPERAND_UNSUPPORTED.into()),
        }
        Ok(())
    }

    fn instr_trb(&self, arg: Operand, state: &mut State) -> Result<(), Error> {
        match arg {
            Operand::Address(addr) => {
                let mut val = state.bus.read(addr);
                self.update_zero_flag(val & state.registers.accumulator, state);
                val &= !state.registers.accumulator;
                state.bus.write(addr, val);
            }
            _ => return Err(OPERAND_UNSUPPORTED.into()),
        }
        Ok(())
    }

    fn instr_bit(&self, arg: Operand, state: &mut State) -> Result<(), Error> {
        let val = match arg {
            Operand::Address(addr) => state.bus.read(addr),
            Operand::Immediate(val) => val as u8,
            _ => return Err(OPERAND_UNSUPPORTED.into()),
        };

        // 65c02, immediate mode only affects zero
        let zero_only = match arg {
            Operand::Immediate(_) => true,
            _ => false,
        };

        if !zero_only {
            state.registers.status.negative = (val & (1 << 7)) != 0;
            state.registers.status.overflow = (val & (1 << 6)) != 0;
        }

        state.registers.status.zero = (val & state.registers.accumulator) == 0;

        Ok(())
    }

    fn instr_cmp(&self, arg: Operand, state: &mut State) -> Result<(), Error> {
        let m = self.get_operand_addr_imm(&arg, state)?;
        self.compare(arg, state, state.registers.accumulator, m)
    }

    fn instr_cpx(&self, arg: Operand, state: &mut State) -> Result<(), Error> {
        let m = self.get_operand_addr_imm(&arg, state)?;
        self.compare(arg, state, state.registers.index_x, m)
    }

    fn instr_cpy(&self, arg: Operand, state: &mut State) -> Result<(), Error> {
        let m = self.get_operand_addr_imm(&arg, state)?;
        self.compare(arg, state, state.registers.index_y, m)
    }

    fn compare(&self, _arg: Operand, state: &mut State, a: u8, b: u8) -> Result<(), Error> {
        let (result, _overflow) = a.overflowing_sub(b);
        state.registers.status.carry = b <= a;
        self.update_negative_flag(result, state);
        self.update_zero_flag(result, state);
        Ok(())
    }

    fn instr_inx(&self, arg: Operand, state: &mut State) -> Result<(), Error> {
        let result = self.increment(arg, state, state.registers.index_x)?;
        state.registers.index_x = result;
        Ok(())
    }

    fn instr_iny(&self, arg: Operand, state: &mut State) -> Result<(), Error> {
        let result = self.increment(arg, state, state.registers.index_y)?;
        state.registers.index_y = result;
        Ok(())
    }

    fn instr_inc(&self, arg: Operand, state: &mut State) -> Result<(), Error> {
        let addr = match arg {
            Operand::Address(addr) => addr,
            Operand::Accumulator => {
                let result = self.increment(arg, state, state.registers.accumulator)?;
                state.registers.accumulator = result;
                return Ok(());
            }
            _ => return Err(OPERAND_UNSUPPORTED.into()),
        };

        let val = state.bus.read(addr);

        // extra read for additional cycle
        let _ = state.bus.read(addr);

        let result = self.increment(arg, state, val)?;
        state.bus.write(addr, result);

        Ok(())
    }

    fn instr_dex(&self, arg: Operand, state: &mut State) -> Result<(), Error> {
        let result = self.decrement(arg, state, state.registers.index_x)?;
        state.registers.index_x = result;
        Ok(())
    }

    fn instr_dey(&self, arg: Operand, state: &mut State) -> Result<(), Error> {
        let result = self.decrement(arg, state, state.registers.index_y)?;
        state.registers.index_y = result;
        Ok(())
    }

    fn instr_dec(&self, arg: Operand, state: &mut State) -> Result<(), Error> {
        let addr = match arg {
            Operand::Address(addr) => addr,
            Operand::Accumulator => {
                let result = self.decrement(arg, state, state.registers.accumulator)?;
                state.registers.accumulator = result;
                return Ok(());
            }
            _ => return Err(OPERAND_UNSUPPORTED.into()),
        };

        let val = state.bus.read(addr);

        // extra read for additional cycle
        let _ = state.bus.read(addr);

        let result = self.decrement(arg, state, val)?;
        state.bus.write(addr, result);

        Ok(())
    }

    fn increment(&self, _arg: Operand, state: &mut State, a: u8) -> Result<u8, Error> {
        let result = a.wrapping_add(1);
        self.update_negative_flag(result, state);
        self.update_zero_flag(result, state);
        Ok(result)
    }

    fn decrement(&self, _arg: Operand, state: &mut State, a: u8) -> Result<u8, Error> {
        let result = a.wrapping_sub(1);
        self.update_negative_flag(result, state);
        self.update_zero_flag(result, state);
        Ok(result)
    }

    fn instr_beq(&self, arg: Operand, state: &mut State) -> Result<(), Error> {
        if state.registers.status.zero {
            self.instr_branch(arg, state)?;
        }
        Ok(())
    }

    fn instr_bcs(&self, arg: Operand, state: &mut State) -> Result<(), Error> {
        if state.registers.status.carry {
            self.instr_branch(arg, state)?;
        }
        Ok(())
    }

    fn instr_branch(&self, arg: Operand, state: &mut State) -> Result<(), Error> {
        match arg {
            Operand::Address(addr) => {
                if addr >> 8 != (state.registers.pc >> 8) {
                    self.inc_cycles(2);
                } else {
                    self.inc_cycles(1);
                }
                state.registers.pc = addr;
            }
            _ => return Err(OPERAND_UNSUPPORTED.into()),
        }
        Ok(())
    }

    fn instr_lsr(&self, arg: Operand, state: &mut State) -> Result<(), Error> {
        match arg {
            Operand::Address(addr) => {
                let mut val = state.bus.read(addr);
                state.registers.status.carry = val & 1 != 0;
                val >>= 1;
                self.update_negative_flag(val, state);
                self.update_zero_flag(val, state);
                state.bus.write(addr, val);
                Ok(())
            }
            Operand::Accumulator => {
                state.registers.status.carry = state.registers.accumulator & 1 != 0;
                state.registers.accumulator >>= 1;
                self.update_negative_flag(state.registers.accumulator, state);
                self.update_zero_flag(state.registers.accumulator, state);
                Ok(())
            }
            _ => Err(OPERAND_UNSUPPORTED.into()),
        }
    }

    fn instr_asl(&self, arg: Operand, state: &mut State) -> Result<(), Error> {
        match arg {
            Operand::Address(addr) => {
                let mut val = state.bus.read(addr);
                state.registers.status.carry = (val >> 7) & 1 != 0;
                val <<= 1;
                self.update_negative_flag(val, state);
                self.update_zero_flag(val, state);
                state.bus.write(addr, val);
                Ok(())
            }
            Operand::Accumulator => {
                state.registers.status.carry = (state.registers.accumulator >> 7) & 1 != 0;
                state.registers.accumulator <<= 1;
                self.update_negative_flag(state.registers.accumulator, state);
                self.update_zero_flag(state.registers.accumulator, state);
                Ok(())
            }
            _ => Err(OPERAND_UNSUPPORTED.into()),
        }
    }

    fn instr_and(&self, arg: Operand, state: &mut State) -> Result<(), Error> {
        state.registers.accumulator &= self.get_operand_addr_imm(&arg, state)?;
        self.update_negative_flag(state.registers.accumulator, state);
        self.update_zero_flag(state.registers.accumulator, state);
        Ok(())
    }

    fn instr_pha(&self, _arg: Operand, state: &mut State) -> Result<(), Error> {
        state.push_sp(state.registers.accumulator);
        Ok(())
    }

    fn instr_pla(&self, _arg: Operand, state: &mut State) -> Result<(), Error> {
        let accum = state.pop_sp();
        self.update_negative_flag(accum, state);
        self.update_zero_flag(accum, state);
        state.registers.accumulator = accum;
        Ok(())
    }

    fn instr_phx(&self, _arg: Operand, state: &mut State) -> Result<(), Error> {
        state.push_sp(state.registers.index_x);
        Ok(())
    }

    fn instr_phy(&self, _arg: Operand, state: &mut State) -> Result<(), Error> {
        state.push_sp(state.registers.index_y);
        Ok(())
    }

    fn instr_php(&self, _arg: Operand, state: &mut State) -> Result<(), Error> {
        let status: u8 = state.registers.status.into();
        state.push_sp(status | (1 << 4)); // Set break flag
        Ok(())
    }

    fn instr_plp(&self, _arg: Operand, state: &mut State) -> Result<(), Error> {
        let value = state.pop_sp();
        state.registers.status = Status::from(value);
        Ok(())
    }

    fn instr_ply(&self, _arg: Operand, state: &mut State) -> Result<(), Error> {
        let value = state.pop_sp();
        state.registers.index_y = value;
        self.update_negative_flag(value, state);
        self.update_zero_flag(value, state);
        Ok(())
    }

    fn instr_plx(&self, _arg: Operand, state: &mut State) -> Result<(), Error> {
        let value = state.pop_sp();
        state.registers.index_x = value;
        self.update_negative_flag(value, state);
        self.update_zero_flag(value, state);
        Ok(())
    }

    fn instr_rti(&self, _arg: Operand, state: &mut State) -> Result<(), Error> {
        state.registers.status = state.pop_sp().into();
        state.registers.pc = state.pop_sp_u16();
        Ok(())
    }

    fn instr_rts(&self, _arg: Operand, state: &mut State) -> Result<(), Error> {
        state.stack_frame.pop();
        state.registers.pc = state.pop_sp_u16() + 1;
        Ok(())
    }

    fn instr_sec(&self, _arg: Operand, state: &mut State) -> Result<(), Error> {
        state.registers.status.carry = true;
        Ok(())
    }

    fn instr_sed(&self, _arg: Operand, state: &mut State) -> Result<(), Error> {
        state.registers.status.decimal_mode = true;
        Ok(())
    }

    fn instr_sei(&self, _arg: Operand, state: &mut State) -> Result<(), Error> {
        state.registers.status.irq_disable = true;
        Ok(())
    }

    fn instr_stp(&self, _arg: Operand, _state: &mut State) -> Result<(), Error> {
        *self.execution_request.lock().unwrap() = ExecutionRequest::Halt;
        Ok(())
    }

    fn instr_stz(&self, arg: Operand, state: &mut State) -> Result<(), Error> {
        match arg {
            Operand::Address(addr) => {
                state.bus.write(addr, 0x00);
            }
            _ => return Err("unsupported addressing mode".into()),
        }
        Ok(())
    }

    fn instr_sta(&self, arg: Operand, state: &mut State) -> Result<(), Error> {
        let addr = self.get_operand_addr(&arg, state)?;
        state.bus.write(addr, state.registers.accumulator);
        Ok(())
    }

    fn instr_stx(&self, arg: Operand, state: &mut State) -> Result<(), Error> {
        let addr = self.get_operand_addr(&arg, state)?;
        state.bus.write(addr, state.registers.index_x);
        Ok(())
    }

    fn instr_sty(&self, arg: Operand, state: &mut State) -> Result<(), Error> {
        let addr = self.get_operand_addr(&arg, state)?;
        state.bus.write(addr, state.registers.index_y);
        Ok(())
    }

    fn instr_tax(&self, _arg: Operand, state: &mut State) -> Result<(), Error> {
        state.registers.index_x = state.registers.accumulator;
        self.update_negative_flag(state.registers.index_x, state);
        self.update_zero_flag(state.registers.index_x, state);
        Ok(())
    }

    fn instr_tay(&self, _arg: Operand, state: &mut State) -> Result<(), Error> {
        state.registers.index_y = state.registers.accumulator;
        self.update_negative_flag(state.registers.index_y, state);
        self.update_zero_flag(state.registers.index_y, state);
        Ok(())
    }

    fn instr_tsx(&self, _arg: Operand, state: &mut State) -> Result<(), Error> {
        state.registers.index_x = state.registers.sp;
        self.update_negative_flag(state.registers.index_x, state);
        self.update_zero_flag(state.registers.index_x, state);
        Ok(())
    }

    fn instr_txa(&self, _arg: Operand, state: &mut State) -> Result<(), Error> {
        state.registers.accumulator = state.registers.index_x;
        self.update_negative_flag(state.registers.accumulator, state);
        self.update_zero_flag(state.registers.accumulator, state);
        Ok(())
    }

    fn instr_txs(&self, _arg: Operand, state: &mut State) -> Result<(), Error> {
        state.registers.sp = state.registers.index_x;
        Ok(())
    }

    fn instr_tya(&self, _arg: Operand, state: &mut State) -> Result<(), Error> {
        state.registers.accumulator = state.registers.index_y;
        self.update_negative_flag(state.registers.accumulator, state);
        self.update_zero_flag(state.registers.accumulator, state);
        Ok(())
    }

    fn instr_ror(&self, arg: Operand, state: &mut State) -> Result<(), Error> {
        match arg {
            Operand::Address(addr) => {
                let mut val = state.bus.read(addr);
                let carry = state.registers.status.carry;
                state.registers.status.carry = (val & 1) != 0;
                val >>= 1;
                if carry {
                    val |= 1 << 7;
                }
                self.update_negative_flag(val, state);
                self.update_zero_flag(val, state);
                state.bus.write(addr, val);
            }
            Operand::Accumulator => {
                let carry = state.registers.status.carry;
                state.registers.status.carry = (state.registers.accumulator & 1) != 0;
                state.registers.accumulator >>= 1;
                if carry {
                    state.registers.accumulator |= 1 << 7;
                }
                self.update_negative_flag(state.registers.accumulator, state);
                self.update_zero_flag(state.registers.accumulator, state);
            }
            _ => return Err(OPERAND_UNSUPPORTED.into()),
        }
        Ok(())
    }

    fn instr_rol(&self, arg: Operand, state: &mut State) -> Result<(), Error> {
        let carry = state.registers.status.carry;
        match arg {
            Operand::Address(addr) => {
                let mut val = state.bus.read(addr);
                state.registers.status.carry = (val & (1 << 7)) != 0;
                val <<= 1;
                if carry {
                    val |= 1;
                }
                self.update_negative_flag(val, state);
                self.update_zero_flag(val, state);
                state.bus.write(addr, val);
            }
            Operand::Accumulator => {
                state.registers.status.carry = (state.registers.accumulator & (1 << 7)) != 0;
                state.registers.accumulator <<= 1;
                if carry {
                    state.registers.accumulator |= 1;
                }
                self.update_negative_flag(state.registers.accumulator, state);
                self.update_zero_flag(state.registers.accumulator, state);
            }
            _ => return Err(OPERAND_UNSUPPORTED.into()),
        }
        Ok(())
    }

    fn instr_ora(&self, arg: Operand, state: &mut State) -> Result<(), Error> {
        let val = self.get_operand_addr_imm(&arg, state)?;
        state.registers.accumulator |= val;
        self.update_negative_flag(state.registers.accumulator, state);
        self.update_zero_flag(state.registers.accumulator, state);
        Ok(())
    }

    fn instr_eor(&self, arg: Operand, state: &mut State) -> Result<(), Error> {
        let val = self.get_operand_addr_imm(&arg, state)?;
        state.registers.accumulator ^= val;
        self.update_negative_flag(state.registers.accumulator, state);
        self.update_zero_flag(state.registers.accumulator, state);
        Ok(())
    }

    fn instr_jmp(&self, arg: Operand, state: &mut State) -> Result<(), Error> {
        match arg {
            Operand::Address(addr) => {
                state.registers.pc = addr;
            }
            _ => return Err("unsupported operand type".into()),
        }
        Ok(())
    }

    fn instr_jsr(&self, arg: Operand, state: &mut State, pc: u16) -> Result<(), Error> {
        match arg {
            Operand::Address(addr) => {
                // address of last byte of jsr
                state.stack_frame.push(pc);
                state.push_sp_u16(state.registers.pc - 1);
                state.registers.pc = addr;
            }
            _ => return Err("unsupported operand type".into()),
        }
        Ok(())
    }
}

impl Debuggable for Mos6502 {
    fn run<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>> {
        Box::pin(async move {
            *self.execution_request.lock().unwrap() = ExecutionRequest::Resume;
            Ok(())
        })
    }

    fn get_state(&self) -> Result<crate::debug::State, crate::errors::Error> {
        let state = self.state.lock().unwrap();

        let mut registers = vec![];
        registers.push(debug::Register {
            name: "A".to_owned(),
            value: state.registers.accumulator as usize,
            size: debug::RegisterSize::Size8,
        });
        registers.push(debug::Register {
            name: "IDX_X".to_owned(),
            value: state.registers.index_x as usize,
            size: debug::RegisterSize::Size8,
        });
        registers.push(debug::Register {
            name: "IDX_Y".to_owned(),
            value: state.registers.index_y as usize,
            size: debug::RegisterSize::Size8,
        });
        registers.push(debug::Register {
            name: "SP".to_owned(),
            value: state.registers.sp as usize,
            size: debug::RegisterSize::Size8,
        });

        let flags = vec![
            ("B".to_owned(), state.registers.status.brk_command),
            ("C".to_owned(), state.registers.status.carry),
            ("Z".to_owned(), state.registers.status.zero),
            ("I".to_owned(), state.registers.status.irq_disable),
            ("D".to_owned(), state.registers.status.decimal_mode),
            ("V".to_owned(), state.registers.status.overflow),
            ("N".to_owned(), state.registers.status.negative),
        ];

        Ok(debug::State {
            registers,
            flags,
            pc: debug::Register {
                name: "PC".to_owned(),
                value: state.registers.pc as usize,
                size: debug::RegisterSize::Size16,
            },
            execution_state: state.execution_state,
        })
    }

    fn step(&self) -> Result<(), crate::errors::Error> {
        self.execute_single()
    }

    fn add_breakpoint(&self, address: usize) -> Result<(), crate::errors::Error> {
        self.state.lock().unwrap().breakpoints.push(address as u16);
        Ok(())
    }

    fn list_breakpoints(&self) -> Result<Vec<usize>, crate::errors::Error> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .breakpoints
            .iter()
            .map(|bp| *bp as usize)
            .collect())
    }

    fn delete_breakpoints(&self, address: usize) -> Result<(), crate::errors::Error> {
        self.state
            .lock()
            .unwrap()
            .breakpoints
            .retain(|bp| *bp as usize != address);
        Ok(())
    }

    fn halt(&self) -> Result<(), Error> {
        *self.execution_request.lock().unwrap() = ExecutionRequest::Halt;
        Ok(())
    }

    fn get_instructions(
        &self,
        address: usize,
        num: usize,
    ) -> Result<Vec<debug::InstructionInfo>, Error> {
        let state = self.state.lock().unwrap();

        let mut instructions = vec![];
        let mut pc = address as u16;
        for _ in 0..num {
            let val = state.bus.read(pc);
            if let Some(opcode) = decode_instruction(val) {
                let (arg, arg_len) = match opcode.opcode().mode {
                    opcodes::AddressingMode::Immediate => {
                        let val = state.bus.read(pc + 1);
                        (format!("#${:x}", val), 1)
                    }
                    opcodes::AddressingMode::Absolute => {
                        let addr = state.bus.read_u16(pc + 1);
                        (format!("${:04x}", addr), 2)
                    }
                    opcodes::AddressingMode::ZeroPage => {
                        let val = state.bus.read(pc + 1);
                        (format!("{:02x?}", val), 1)
                    }
                    opcodes::AddressingMode::Accum => ("".to_owned(), 0),
                    opcodes::AddressingMode::Implied => ("".to_owned(), 0),
                    opcodes::AddressingMode::IndexX => {
                        (format!("({:02x?},X)", state.bus.read(pc + 1)), 1)
                    }
                    opcodes::AddressingMode::IndexY => {
                        (format!("({:02x?}),Y", state.bus.read(pc + 1)), 1)
                    }
                    opcodes::AddressingMode::ZeroPageX => {
                        (format!("{:02x?},X", state.bus.read(pc + 1)), 1)
                    }
                    opcodes::AddressingMode::ZeroPageY => {
                        (format!("{:02x?},Y", state.bus.read(pc + 1)), 1)
                    }
                    opcodes::AddressingMode::AbsX => {
                        (format!("{:04x?},X", state.bus.read_u16(pc + 1)), 2)
                    }
                    opcodes::AddressingMode::AbsY => {
                        (format!("{:04x?},Y", state.bus.read_u16(pc + 1)), 2)
                    }
                    opcodes::AddressingMode::Relative => {
                        let val = state.bus.read(pc + 1) as i8;
                        (format!("{:x?}", ((pc as i16 + 2) + (val as i16)) as u16), 1)
                    }
                    opcodes::AddressingMode::Indirect => {
                        (format!("({:x?})", state.bus.read_u16(pc + 1)), 2)
                    }
                    opcodes::AddressingMode::ZeroPageIndirect => {
                        let val = state.bus.read(pc + 1);
                        (format!("(zp({:02x?}))", val), 1)
                    }
                    opcodes::AddressingMode::ZeroPageRel => {
                        let val = state.bus.read(pc + 1);
                        (
                            format!("zp({:02x?}), {:02x?}", val, state.bus.read(pc + 2)),
                            2,
                        )
                    }
                    opcodes::AddressingMode::Nop2 => ("".to_owned(), 1),
                    opcodes::AddressingMode::Nop3 => ("".to_owned(), 2),
                    opcodes::AddressingMode::AbsXIndirect => {
                        (format!("({:02x?},X)", state.bus.read_u16(pc + 1)), 2)
                    }
                };

                instructions.push(InstructionInfo {
                    address: pc as usize,
                    instruction: format!("{} {}", opcode.opcode().name.to_owned(), arg),
                    opcode: (pc..pc + 1 + arg_len)
                        .map(|addr| state.bus.read(addr))
                        .collect::<Vec<_>>(),
                });

                pc += 1 + arg_len;
            } else {
                break;
            }
        }

        Ok(instructions)
    }

    fn read_memory(&self, address: usize, num: usize) -> Result<Vec<u8>, Error> {
        let s = self.state.lock().unwrap();
        Ok((0..num).map(|n| s.bus.read((address + n) as u16)).collect())
    }

    fn backtrace(&self) -> Result<Vec<usize>, Error> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .stack_frame
            .iter()
            .map(|s| *s as usize)
            .collect())
    }

    fn add_memory_breakpoint(&self, _address: usize) -> Result<(), Error> {
        Ok(())
    }

    fn list_memory_breakpoints(&self) -> Result<Vec<usize>, Error> {
        Ok(vec![])
    }

    fn delete_memory_breakpoint(&self, _address: usize) -> Result<(), Error> {
        Ok(())
    }
}

impl InterruptTarget for Mos6502 {
    fn trigger_irq(&self, nmi: bool, source: super::InterruptSource) {
        self.irq_trigger(nmi, source);
    }

    fn release_irq(&self, nmi: bool, source: super::InterruptSource) {
        self.irq_release(nmi, source);
    }
}

impl Clock for Mos6502 {
    fn elapsed(&self) -> crate::clock::ClockInstant {
        let cycles = self.get_cycle_count();
        ClockInstant {
            instant: cycles,
            tick_duration: self.clock_cycle_duration,
        }
    }

    fn add_tick_listener(&self, listener: Box<dyn crate::clock::ClockTickListener>) {
        self.state.lock().unwrap().tick_listeners.push(listener);
    }
}

#[cfg(test)]
pub mod tests {
    use std::{panic, sync::Arc, u64};

    use crate::{
        cpu::mos6502::{
            ClockConfig, Mos6502,
            bus::{Bus, memory::MemoryBank},
        },
        debug::Debuggable as _,
    };

    fn test_instruction(opcode: &[u8]) -> Mos6502 {
        let _ = env_logger::try_init();
        let mut data = vec![0u8; 0xffff];

        data[0xff00..0xff00 + opcode.len()].copy_from_slice(opcode);

        let mem = Arc::new(MemoryBank::new_with_data(&data, false));
        let bus = Arc::new(Bus::new());
        bus.connect(0x0, 0xffff, mem);
        let cpu = Mos6502::new(
            bus,
            ClockConfig::new(u64::MAX, 1.0),
            crate::cpu::mos6502::Variant::Original,
        );
        cpu.state.lock().unwrap().registers.pc = 0xff00;

        cpu
    }

    #[test]
    fn test_ldy_zp_x() {
        let cpu = test_instruction(&[0xb4, 0x10]);
        cpu.state.lock().unwrap().registers.index_x = 0x10;
        cpu.state.lock().unwrap().bus.write(0x20, 0xff);
        cpu.execute_single().unwrap();
        let s = cpu.state.lock().unwrap();
        assert_eq!(0xff, s.registers.index_y);
        assert_eq!(false, s.registers.status.zero);
        assert_eq!(true, s.registers.status.negative);
    }

    #[test]
    fn test_ldy() {
        let cpu = test_instruction(&[0xa0, 0]);
        cpu.execute_single().unwrap();
        let s = cpu.state.lock().unwrap();
        assert_eq!(0, s.registers.index_y);
        assert_eq!(true, s.registers.status.zero);
        assert_eq!(false, s.registers.status.negative);
    }

    #[test]
    fn test_ldy_negative() {
        let cpu = test_instruction(&[0xa0, 0xff]);
        cpu.execute_single().unwrap();
        let s = cpu.state.lock().unwrap();
        assert_eq!(0xff, s.registers.index_y);
        assert_eq!(false, s.registers.status.zero);
        assert_eq!(true, s.registers.status.negative);
    }

    #[test]
    fn test_adc_carry() {
        let cpu = test_instruction(&[0x69, 6]);
        cpu.state.lock().unwrap().registers.accumulator = 254;
        cpu.state.lock().unwrap().registers.status.carry = true;
        cpu.execute_single().unwrap();
        let s = cpu.state.lock().unwrap();
        assert_eq!(5, s.registers.accumulator);
        assert_eq!(false, s.registers.status.zero);
        assert_eq!(true, s.registers.status.carry);
        assert_eq!(false, s.registers.status.negative);
        assert_eq!(false, s.registers.status.overflow);
    }

    #[test]
    fn test_adc_overflow_positive() {
        let cpu = test_instruction(&[0x69, 100]);
        cpu.state.lock().unwrap().registers.accumulator = 100;
        cpu.state.lock().unwrap().registers.status.carry = false;
        cpu.execute_single().unwrap();
        let s = cpu.state.lock().unwrap();
        assert_eq!(200, s.registers.accumulator);
        assert_eq!(false, s.registers.status.zero);
        assert_eq!(false, s.registers.status.carry);
        assert_eq!(true, s.registers.status.negative);
        assert_eq!(true, s.registers.status.overflow);
    }

    #[test]
    fn test_65c02() {
        let _ = env_logger::try_init();
        let data = std::fs::read("test/65C02_extended_opcodes_test.bin").unwrap();
        assert_eq!(65536, data.len());
        let mem = Arc::new(MemoryBank::new_with_data(&data, false));
        let bus = Arc::new(Bus::new());
        bus.connect(0x0, 0x10000, mem);
        let cpu = Mos6502::new(
            bus,
            ClockConfig::new(u64::MAX, 1.0),
            crate::cpu::mos6502::Variant::Wdc65c02,
        );
        cpu.state.lock().unwrap().registers.pc = 0x400;

        let mut pcs = vec![];
        loop {
            let pc = cpu.state.lock().unwrap().registers.pc;
            pcs.push(pc);
            if pcs.len() > 50 {
                pcs.remove(0);
            }

            /*println!(
                "{}",
                cpu.get_instructions(pc as usize, 1)
                    .unwrap()
                    .first()
                    .unwrap()
            );*/

            cpu.execute_single().unwrap();

            // all tests passsed
            if cpu.state.lock().unwrap().registers.pc == 0x24f1 {
                return;
            }

            if cpu.state.lock().unwrap().registers.pc == pc {
                println!("state: {:x?}", cpu.state.lock().unwrap().registers);
                println!(
                    "{}",
                    pcs.iter()
                        .map(|pc| {
                            if let Some(op) = cpu.get_instructions(*pc as usize, 1).unwrap().first()
                            {
                                format!("{}", op)
                            } else {
                                "unknown".to_owned()
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                );
                panic!("test failed");
            }
        }
    }

    #[test]
    fn test_all_opcodes() {
        let _ = env_logger::try_init();
        let data = std::fs::read("test/6502_functional_test.bin").unwrap();
        assert_eq!(65536, data.len());
        let mem = Arc::new(MemoryBank::new_with_data(&data, false));
        let bus = Arc::new(Bus::new());
        bus.connect(0x0, 0x10000, mem);
        let cpu = Mos6502::new(
            bus,
            ClockConfig::new(u64::MAX, 1.0),
            crate::cpu::mos6502::Variant::Original,
        );
        cpu.state.lock().unwrap().registers.pc = 0x400;

        let mut pcs = vec![];
        loop {
            let pc = cpu.state.lock().unwrap().registers.pc;
            pcs.push(pc);
            if pcs.len() > 50 {
                pcs.remove(0);
            }

            cpu.execute_single().unwrap();

            // all tests passsed
            if cpu.state.lock().unwrap().registers.pc == 0x3469 {
                return;
            }

            if cpu.state.lock().unwrap().registers.pc == pc {
                println!("state: {:x?}", cpu.state.lock().unwrap().registers);
                println!(
                    "{}",
                    pcs.iter()
                        .map(|pc| {
                            if let Some(op) = cpu.get_instructions(*pc as usize, 1).unwrap().first()
                            {
                                format!("{}", op)
                            } else {
                                "unknown".to_owned()
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                );
                panic!("test failed");
            }
        }
    }
}
