use std::path::PathBuf;

/// Builds the owned native tap; its bytes travel with the adapter, never a plugin search path.
fn main() {
    println!("cargo:rerun-if-changed=native/tap.c");
    println!("cargo:rerun-if-changed=native/ladspa/ladspa.h");
    let output = PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo output directory"))
        .join("resolibre-tap.so");
    let compiler = cc::Build::new().get_compiler();
    let status = compiler
        .to_command()
        .args([
            "-std=c11", "-O2", "-fPIC", "-shared", "-Wall", "-Wextra", "-Werror",
        ])
        .arg("native/tap.c")
        .arg("-o")
        .arg(&output)
        .status()
        .expect("execute tap compiler");
    assert!(status.success(), "native tap compilation failed");
    println!("cargo:rustc-env=RESOUL_TAP_BINARY={}", output.display());
}
