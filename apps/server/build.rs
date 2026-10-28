//! Build script for whatsapp-translator.
//!
//! This script optionally compiles the Go wa-bridge binary during `cargo build`.
//! If Go is not installed or the build fails, it will print a warning but not fail the build.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    // Rebuild when anything in the Go bridge changes. Cargo scans a watched
    // directory recursively, so new or renamed Go files are picked up too, and
    // Go's build cache keeps reruns caused by test-only edits cheap.
    println!("cargo:rerun-if-changed=../wa-bridge");

    // Skip Go build if explicitly disabled. Rerun when the variable changes,
    // so a skipped build (as in `make check` and CI) does not stick to later
    // `cargo build` runs.
    println!("cargo:rerun-if-env-changed=SKIP_GO_BUILD");
    if env::var("SKIP_GO_BUILD").is_ok() {
        println!("cargo:warning=Skipping Go bridge build (SKIP_GO_BUILD is set)");
        return;
    }

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let wa_bridge_dir = PathBuf::from(&manifest_dir).join("../wa-bridge");
    let out_dir = env::var("OUT_DIR").unwrap();
    let target_dir = PathBuf::from(&out_dir)
        .ancestors()
        .nth(3)
        .unwrap()
        .to_path_buf();

    // Determine output binary name based on target OS
    let binary_name = if cfg!(target_os = "windows") {
        "wa-bridge.exe"
    } else {
        "wa-bridge"
    };

    let output_path = target_dir.join(binary_name);

    // Check if Go is available
    let go_check = Command::new("go").arg("version").output();

    if go_check.is_err() {
        println!("cargo:warning=Go is not installed. Please build wa-bridge manually:");
        println!("cargo:warning=  cd apps/wa-bridge && go build -o wa-bridge .");
        return;
    }

    // Build the Go binary
    println!("cargo:warning=Building wa-bridge...");

    let status = Command::new("go")
        .args(["build", "-o", output_path.to_str().unwrap(), "."])
        .current_dir(&wa_bridge_dir)
        .status();

    match status {
        Ok(s) if s.success() => {
            println!(
                "cargo:warning=wa-bridge built successfully at {:?}",
                output_path
            );
        }
        Ok(s) => {
            println!(
                "cargo:warning=Failed to build wa-bridge (exit code: {:?})",
                s.code()
            );
            println!("cargo:warning=Please build wa-bridge manually:");
            println!("cargo:warning=  cd apps/wa-bridge && go build -o wa-bridge .");
        }
        Err(e) => {
            println!("cargo:warning=Failed to run go build: {}", e);
            println!("cargo:warning=Please build wa-bridge manually:");
            println!("cargo:warning=  cd apps/wa-bridge && go build -o wa-bridge .");
        }
    }
}
