//! Emits the value the checked function compiles against. Editing this file
//! changes what that function returns with nothing under `src/` touched --
//! which is the point of the fixture.
fn main() {
    println!("cargo:rustc-env=PLY_FIXTURE_ANSWER=7");
    println!("cargo:rerun-if-changed=build.rs");
}
