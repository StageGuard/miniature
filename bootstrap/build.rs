
const VA_BASE: u64 = 0xffff_8000_0000_0000;

fn main() {
    println!("cargo:rustc-link-arg=--image-base={}", VA_BASE);
}