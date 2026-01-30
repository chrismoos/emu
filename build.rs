use std::{env, process::Command};

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();

    // Compile custom Apple II assembly files
    let appleii_asm = ["mouse", "smartport"];
    for f in appleii_asm {
        if !Command::new("ca65")
            .args([
                "--relax-checks",
                "-t",
                "apple2",
                &format!("resources/appleii/asm/{}.a65", f),
                "-o",
                &format!("{}/{}.o", out_dir, f),
            ])
            .status()
            .unwrap()
            .success()
        {
            panic!("failed to compile {}", f);
        }
        if !Command::new("ld65")
            .args([
                "-C",
                &format!("resources/appleii/asm/{}.cfg", f),
                "-o",
                &format!("{}/appleii-{}.bin", out_dir, f),
                &format!("{}/{}.o", out_dir, f),
            ])
            .status()
            .unwrap()
            .success()
        {
            panic!("failed to link {}", f);
        }
    }
}
