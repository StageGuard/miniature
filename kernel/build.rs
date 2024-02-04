const KERNEL_VA_BASE: u64 = 0xffff_ff80_0000_0000;

fn main() {
    println!("cargo:rustc-link-arg=--image-base={}", KERNEL_VA_BASE);
}