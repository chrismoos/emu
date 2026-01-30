#![deny(warnings)]

use clap::{Parser, Subcommand};
use emu::emulator::app::EmulatorApplication;
use emu::{
    emulator::{Config, Emulator},
    errors::Error,
    targets::appleii::{
        self,
        args::{AppleIIEEnhancedArgs, RunArgs, Variant},
    },
};
use log::info;
use std::sync::Arc;

/// emu is an emulator.
#[derive(Parser)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Emulation Target (defaults to Apple IIe Enhanced if not specified)
    #[command(subcommand)]
    target: Option<EmulationTarget>,

    /// Halt target at launch (do not start automatically)
    #[arg(long, default_value_t = false)]
    halt_on_start: bool,
}

#[derive(Subcommand, Clone)]
pub enum EmulationTarget {
    /// AppleII
    AppleII(appleii::args::RunArgs),
}

fn default_run_args() -> RunArgs {
    let mut run_args = RunArgs::default();
    run_args.variant = Variant::AppleIIEEnhanced(AppleIIEEnhancedArgs::default());
    run_args
}

fn main() -> Result<(), Error> {
    #[cfg(feature = "native")]
    {
        env_logger::builder().format_timestamp_micros().init();
        let args = Args::parse();
        let run_args = match args.target {
            Some(EmulationTarget::AppleII(run_args)) => run_args,
            None => default_run_args(),
        };
        let target = appleii::target::Target::new(run_args)?;
        info!("Starting...");

        EmulatorApplication::new(Emulator::new(
            Arc::new(target),
            Config {
                halt_at_start: args.halt_on_start,
            },
        ))
        .start()
    }

    #[cfg(feature = "wasm")]
    {
        console_log::init_with_level(log::Level::Debug)?;
        let run_args = default_run_args();
        info!("Starting...");

        EmulatorApplication::new(Emulator::new(
            Arc::new(appleii::target::Target::new(run_args)?),
            Config {
                // Needs to be true, if we auto-start the play audio fails for now
                halt_at_start: true,
            },
        ))
        .start()
    }
}
