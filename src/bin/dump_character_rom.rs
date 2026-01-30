use std::{
    fs::File,
    io::Read,
};

use clap::Parser;

use emu::errors::Error;

/// emu is an emulator.
#[derive(Parser)]
#[command(version, about, long_about = None)]
pub struct Args {
    #[arg(short, long)]
    rom_file: String,

    #[arg(short, long, default_value_t = false)]
    reverse_bits: bool,
}

pub fn main() -> Result<(), Error> {
    let args = Args::parse();
    println!("Dumping ROM File: {}", args.rom_file);

    let mut buf = vec![];
    File::open(&args.rom_file)?.read_to_end(&mut buf)?;

    let mut char_index = 0;
    buf.chunks_exact(8).for_each(|chunk| {
        println!("{}", (0..32).map(|_| "-").collect::<Vec<_>>().join(""));
        println!("Character {:x}", char_index);
        println!("{}", (0..32).map(|_| "-").collect::<Vec<_>>().join(""));
        chunk
            .iter()
            .map(|b| {
                format!(
                    "{:08b}",
                    if args.reverse_bits {
                        b.reverse_bits()
                    } else {
                        *b
                    }
                )
                .replace("1", "*")
                .replace("0", " ")
            })
            .for_each(|l| println!("{}", l));
        char_index += 1;
    });

    Ok(())
}
