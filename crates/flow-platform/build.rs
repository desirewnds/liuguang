// SPDX-License-Identifier: Apache-2.0
//! Compile-time resource input for native macOS packages. Windows is unchanged.
use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=FLOW_MACOS_RESOURCE_MANIFEST");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let path = PathBuf::from(env::var_os("FLOW_MACOS_RESOURCE_MANIFEST")
        .expect("Prepare the macOS engines first, then set FLOW_MACOS_RESOURCE_MANIFEST to their absolute engine-manifest.json path"));
    assert!(path.is_absolute(), "The macOS resource manifest must be an absolute path");
    println!("cargo:rerun-if-changed={}", path.display());
    let text = fs::read_to_string(&path).expect("Read macOS resource manifest");
    // Lightweight routing guard only. The preparation/packaging programs parse
    // and validate the complete JSON and each file hash; this guard does not
    // claim to be a JSON parser or an integrity/security verifier.
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let target = env::var("TARGET").expect("Cargo target");
    assert!(compact.contains(&format!("\"target\":\"{target}\"")),
        "The resource manifest belongs to a different target");
    assert!(compact.contains("\"schema\":1") && compact.contains("\"files\":["),
        "Missing resource manifest fields; use scripts/prepare-macos-engines.py");
}
