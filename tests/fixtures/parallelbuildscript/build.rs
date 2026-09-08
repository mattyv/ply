use std::fs::OpenOptions;
use std::time::Duration;

fn main() {
    let marker = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join(".generator-running");
    let _held = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&marker)
        .expect("two build scripts wrote the package source concurrently");
    std::thread::sleep(Duration::from_millis(200));
    std::fs::remove_file(marker).unwrap();
    println!("cargo:rerun-if-changed=build.rs");
}
