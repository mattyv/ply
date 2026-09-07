use std::fs;

fn main() {
    let base: u32 = fs::read_to_string("answer.txt")
        .expect("answer.txt")
        .trim()
        .parse()
        .expect("an integer in answer.txt");
    let offset: u32 = std::env::var("PLY_FIXTURE_OFFSET")
        .unwrap_or_else(|_| "0".to_string())
        .parse()
        .expect("an integer in PLY_FIXTURE_OFFSET");
    println!("cargo:rustc-env=PLY_FIXTURE_ANSWER={}", base + offset);
    println!("cargo:rerun-if-changed=answer.txt");
    println!("cargo:rerun-if-env-changed=PLY_FIXTURE_OFFSET");
}
