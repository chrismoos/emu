use std::{
    io::{Read, Seek},
    pin::Pin,
    sync::{Arc, Mutex},
};

use eframe::egui::{self, Key, TextEdit, Ui, Widget, vec2};
use log::debug;

use crate::{
    debug::Debuggable,
    errors::Error,
    targets::appleii::{self},
    utils::futures::spawn,
};

pub mod app;
pub mod debug;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum ResetType {
    Cold,
    Warm,
}
pub struct Stats {
    pub fps: Option<f32>,
    pub cpu_actual_freq: f64,
}

pub trait EmulatorProgramReader: Read + Seek + Send {}
impl<R> EmulatorProgramReader for R where R: Read + Seek + Send {}

pub trait EmulatorTarget: Debuggable + Send + Sync {
    fn start<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>>;
    fn stop<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>>;
    fn reset<'a>(
        &'a self,
        reset_type: ResetType,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>>;
    fn update_display(&self, ui: &mut Ui, ctx: &egui::Context, frame: &mut eframe::Frame);
    fn fill_audio_buffer(&self, data: &mut [f32], channels: usize);
    fn configure_audio(&self, audio_config: &AudioConfig);
    fn stats(&self) -> Stats;
    fn unload_program(&self) -> Result<(), Error>;
    fn load_program(
        &self,
        file_name: &str,
        reader: Box<dyn EmulatorProgramReader>,
    ) -> Result<(), Error>;
    fn target_type_id(&self) -> &str;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmulatorTargetType {
    pub name: &'static str,
    pub id: &'static str,
}

pub trait EmulatorTargetFactory {
    fn name(&self) -> &str;
    fn get_types(&self) -> &[EmulatorTargetType];
    fn create(&self, id: &str) -> Option<Arc<dyn EmulatorTarget>>;
}

const TARGET_FACTORIES: &[&'static dyn EmulatorTargetFactory] =
    &[&appleii::target::TargetFactory::new()];

pub struct Config {
    pub halt_at_start: bool,
}

#[derive(Default)]
struct DebugCliState {
    buffer: String,
    input_buffer: String,
}

pub struct Emulator {
    target: Arc<Mutex<Arc<dyn EmulatorTarget>>>,
    config: Config,
    debug_cli_state: Mutex<DebugCliState>,
}

impl Emulator {
    pub fn new(target: Arc<dyn EmulatorTarget>, config: Config) -> Emulator {
        Emulator {
            target: Arc::new(Mutex::new(target)),
            config,
            debug_cli_state: Mutex::new(DebugCliState::default()),
        }
    }

    pub async fn run(&self) -> Result<(), Error> {
        let t = self.target.clone();
        spawn(async move {
            loop {
                let target = t.lock().unwrap().clone();
                if let Ok(_) = target.start().await {
                    debug!("Target exited normally, restarting...");
                }
            }
        });

        debug!("Target ready.");

        if !self.config.halt_at_start {
            self.target.lock().unwrap().run().await?;
        }

        #[cfg(feature = "native")]
        {
            use tokio::io::AsyncBufReadExt;

            let reader = tokio::io::BufReader::new(tokio::io::stdin());
            let mut lines = reader.lines();

            loop {
                use tokio::io::AsyncWriteExt;

                print!("> ");
                tokio::io::stdout().flush().await?;
                let line = lines.next_line().await?.ok_or("failed to read next line")?;
                if let Err(e) = self.handle_command(line.trim()) {
                    println!("error: {:?}", e);
                }
            }
        }

        #[cfg(not(feature = "native"))]
        Ok(())
    }

    fn write_log<S: Into<String>>(&self, s: S) {
        let val = s.into();
        println!("{}", val);
        let mut state = self.debug_cli_state.lock().unwrap();
        state.buffer += &val;
        state.buffer += "\n";
    }

    fn handle_command(&self, command: &str) -> Result<(), Error> {
        match command {
            "quit" | "q" => return Ok(()),
            "info" | "i" => self.print_regs()?,
            "halt" | "h" => self.target.lock().unwrap().halt()?,
            "bt" => self.backtrace()?,
            // switch to add bp
            d if d == "disas" || d == "d" || d.starts_with("disas ") || d.starts_with("d ") => {
                self.disas(d)?
            }
            m if m.starts_with("mem ") || m.starts_with("m ") => self.print_memory(m)?,
            "continue" | "c" => {
                self.write_log("Resuming target...");
                let target = self.target.lock().unwrap().clone();
                spawn(async move {
                    target.run().await.unwrap();
                });
            }
            "step" | "s" => self.step()?,
            mb if mb.starts_with("memorybreak ") || mb.starts_with("mb ") => {
                self.add_memory_breakpoint(mb)?
            }
            b if b.starts_with("break ") || b.starts_with("b ") => self.add_breakpoint(b)?,
            _ => self.write_log("unknown command"),
        }
        Ok(())
    }

    fn backtrace(&self) -> Result<(), Error> {
        let mut bt = self.target.lock().unwrap().backtrace()?;
        bt.reverse();
        self.write_log("\nBacktrace:\n");
        for addr in bt {
            self.write_log(format!("{:08x}", addr));
        }
        Ok(())
    }

    fn add_memory_breakpoint(&self, breakpoint: &str) -> Result<(), Error> {
        let s = breakpoint
            .split(" ")
            .skip(1)
            .next()
            .ok_or("missing breakpoint address")?;

        let bp = self.parse_integer_literal(s)?;
        self.write_log(format!("adding memory breakpoint at 0x{:08x}", bp));

        self.target.lock().unwrap().add_memory_breakpoint(bp)?;

        Ok(())
    }

    fn add_breakpoint(&self, breakpoint: &str) -> Result<(), Error> {
        let s = breakpoint
            .split(" ")
            .skip(1)
            .next()
            .ok_or("missing breakpoint address")?;

        let bp = self.parse_integer_literal(s)?;
        self.write_log(format!("adding breakpoint at 0x{:08x}", bp));

        self.target.lock().unwrap().add_breakpoint(bp)?;

        Ok(())
    }

    fn print_memory(&self, s: &str) -> Result<(), Error> {
        let sz = s.to_owned();
        let s = s
            .split(" ")
            .skip(1)
            .next()
            .ok_or("missing memory address")?;

        let n = sz
            .split(" ")
            .skip(2)
            .next()
            .map(|s| s.parse::<usize>())
            .unwrap_or(Ok(32))?;

        let mem = self.parse_integer_literal(s)?;

        let memory = self.target.lock().unwrap().read_memory(mem, n)?;
        self.write_log(format!(
            "{}",
            memory
                .iter()
                .map(|m| format!("{:02x}", m))
                .collect::<Vec<_>>()
                .join(" ")
        ));

        Ok(())
    }

    fn parse_integer_literal(&self, s: &str) -> Result<usize, Error> {
        let lower = s.to_lowercase();
        if lower.starts_with("0x") {
            Ok(usize::from_str_radix(lower.trim_start_matches("0x"), 16)?)
        } else {
            Ok(usize::from_str_radix(&lower, 10)?)
        }
    }

    fn step(&self) -> Result<(), Error> {
        self.target.lock().unwrap().step()?;
        let state = self.target.lock().unwrap().get_state()?;
        self.write_log(format!("Execution State: {:?}", state.execution_state));
        self.write_log(format!("{}", state.pc));
        Ok(())
    }

    fn disas(&self, s: &str) -> Result<(), Error> {
        let s = s.split(" ").skip(1).next();
        let state = self.target.lock().unwrap().get_state()?;

        let addr = s
            .map(|s| self.parse_integer_literal(s))
            .transpose()?
            .unwrap_or(state.pc.value);

        self.write_log("");
        for instr in self.target.lock().unwrap().get_instructions(addr, 25)? {
            self.write_log(format!(
                "{:08x}: {:<16}{}",
                instr.address,
                instr
                    .opcode
                    .iter()
                    .map(|s| format!("{:02x}", s))
                    .collect::<Vec::<_>>()
                    .join(" "),
                instr.instruction
            ));
        }

        Ok(())
    }

    fn print_regs(&self) -> Result<(), Error> {
        let state = self.target.lock().unwrap().get_state()?;
        self.write_log(format!("Execution State: {:?}", state.execution_state));
        self.write_log(format!("{}", state.pc));
        self.write_log(format!(
            "Registers: \n{}\n",
            state
                .registers
                .iter()
                .map(|r| format!("{}", r))
                .collect::<Vec<_>>()
                .join("\t")
        ));

        self.write_log(format!(
            "Flags:\n{}\n",
            state
                .flags
                .iter()
                .map(|f| format!("{}: {}", f.0, if f.1 { "1" } else { "0" }))
                .collect::<Vec<_>>()
                .join("  "),
        ));

        self.write_log(format!("{}", state.pc));
        Ok(())
    }
}

impl Widget for &Emulator {
    fn ui(self, ui: &mut Ui) -> egui::Response {
        let mut state = self.debug_cli_state.lock().unwrap();
        ui.vertical(|ui| {
            let input_size = egui::vec2(ui.available_width(), 20.0);

            egui::ScrollArea::vertical()
                .max_height(ui.available_height() - 24.0)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    ui.add(
                        TextEdit::multiline(&mut state.buffer)
                            .min_size(vec2(ui.available_width(), ui.available_height()))
                            .interactive(false),
                    );
                });

            let response =
                ui.add(TextEdit::singleline(&mut state.input_buffer).min_size(input_size));

            if response.has_focus() {
                response.ctx.input_mut(|input| {
                    for key in input.keys_down.clone() {
                        input.consume_key(input.modifiers, key);
                    }
                });
            }

            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                let b = state.input_buffer.to_owned();
                state.input_buffer.clear();
                response.request_focus();
                drop(state);
                self.handle_command(&b).unwrap();
                response.ctx.input_mut(|input| {
                    input.consume_key(input.modifiers, Key::Enter);
                });
            }
        });
        ui.response()
    }
}


pub struct AudioConfig {
    pub sample_rate: u32,
    pub channels: u16,
}