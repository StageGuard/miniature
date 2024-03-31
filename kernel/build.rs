use std::env;
use std::process::Command;

const KERNEL_VA_BASE: u64 = 0xffff_ff80_0000_0000;

fn main() {
    println!("cargo:rustc-link-arg=--image-base={}", KERNEL_VA_BASE);
    println!("cargo:rerun-if-changed=src/asm/trampoline.asm");

    let out_dir = env::var("OUT_DIR").unwrap();

    let status = Command::new("nasm")
        .arg("-f")
        .arg("bin")
        .arg("-o")
        .arg(format!("{}/trampoline", out_dir))
        .arg("src/asm/trampoline.asm")
        .status()
        .expect("failed to run nasm");
    if !status.success() {
        panic!("nasm failed with exit status {}", status);
    }
}