use clap::{Args, Subcommand};

#[derive(Default, Args, Clone, Debug)]
pub struct RunArgs {
    #[command(subcommand)]
    pub variant: Variant,

    #[arg(long)]
    pub program_rom: Option<String>,

    /// Path to a serial port to conenct to the Super Serial Card.
    #[arg(long)]
    pub serial_port: Option<String>,

    /// Workaround for using a local (i.e via socat) serial tty (for testing)
    /// This ignores baud rates set on the card and always uses zero.
    #[arg(long)]
    pub serial_force_zero_baud: Option<bool>,
}

#[derive(Args, Clone, Debug, Default, PartialEq, Eq)]
pub struct AppleIIArgs {
    /// Program for 0xF800-0xFFFF ROM
    #[arg(long)]
    pub rom_f8: Option<String>,

    /// Program for 0xE000-0xE7FF ROM
    #[arg(long)]
    pub rom_e0: Option<String>,

    /// Program for 0xE800-0xEFFF ROM
    #[arg(long)]
    pub rom_e8: Option<String>,

    /// Program for 0xF000-0xF7FF ROM
    #[arg(long)]
    pub rom_f0: Option<String>,

    /// Program for 0xD000-0xD7FF ROM
    #[arg(long)]
    pub rom_d0: Option<String>,

    /// Program for 0xD800-0xDFFF ROM
    #[arg(long)]
    pub rom_d8: Option<String>,
}

#[derive(Subcommand, Clone, Debug, PartialEq, Eq)]
pub enum Variant {
    AppleII(AppleIIArgs),
    AppleIIE(AppleIIEArgs),
    AppleIIEEnhanced(AppleIIEEnhancedArgs),
    AppleIIC(AppleIICArgs),
}

impl Default for Variant {
    fn default() -> Self {
        Variant::AppleII(AppleIIArgs::default())
    }
}

#[derive(Args, Clone, Debug, Default, PartialEq, Eq)]
pub struct AppleIIEArgs {
    /// Program for 0xC000-0xDFFF ROM
    #[arg(long)]
    pub rom_c0: Option<String>,

    /// Program for 0xE000-0xFFFF ROM
    #[arg(long)]
    pub rom_e0: Option<String>,
}

#[derive(Args, Clone, Debug, Default, PartialEq, Eq)]
pub struct AppleIIEEnhancedArgs {
    /// Program for 0xC000-0xDFFF ROM
    #[arg(long)]
    pub rom_c0: Option<String>,

    /// Program for 0xE000-0xFFFF ROM
    #[arg(long)]
    pub rom_e0: Option<String>,
}

#[derive(Args, Clone, Debug, Default, PartialEq, Eq)]
pub struct AppleIICArgs {
    /// Program for 0xC000-0xFFFF ROM
    #[arg(long)]
    pub rom_c0: Option<String>,
}
