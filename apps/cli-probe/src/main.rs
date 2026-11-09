//! G1 CLI smoke test (spec §19). Touches nothing but stdout.

fn main() {
    println!("EINK_READER_RUST_OK");
    println!("arch={}", std::env::consts::ARCH);
    println!("os={}", std::env::consts::OS);
    println!("pointer_width={}", usize::BITS);
    println!("version={}", env!("CARGO_PKG_VERSION"));
}
