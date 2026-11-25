// SPDX-License-Identifier: Apache-2.0
//! Windows executable metadata. No dependency downloads or writes outside OUT_DIR.
use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    let source =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory"));
    let icon = source.join("../../packaging/icon.ico");
    let rc = source.join("resources/flow.rc");
    let manifest = source.join("resources/flow.manifest");
    for path in [&icon, &rc, &manifest] {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    println!("cargo:rerun-if-changed=Cargo.toml");
    for name in [
        "FLOW_WINDRES",
        "CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER",
        "RUSTC_LINKER",
        "PATH",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    if env::var_os("CARGO_FEATURE_DESKTOP").is_none() {
        return;
    }
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        println!("cargo:warning=Windows executable resources are omitted on non-Windows targets; the desktop product is verified for Windows GNU only.");
        return;
    }
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("gnu")
        || env::var("CARGO_CFG_TARGET_ARCH").as_deref() != Ok("x86_64")
    {
        panic!("Flow desktop packaging currently requires x86_64-pc-windows-gnu. MSVC and other resource toolchains are not validated; no resources will be silently omitted.");
    }
    let version = env::var("CARGO_PKG_VERSION").expect("Cargo package version");
    let mut numbers = version
        .split(['-', '+'])
        .next()
        .unwrap_or_default()
        .split('.')
        .map(|n| {
            n.parse::<u16>()
                .expect("Windows version components must fit u16")
        })
        .collect::<Vec<_>>();
    assert_eq!(numbers.len(), 3, "Expected a three-component Cargo version");
    numbers.push(0);
    let numbers = numbers.iter().map(u16::to_string).collect::<Vec<_>>();
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output directory"));
    fs::create_dir_all(&output).expect("Create resource output directory");
    fs::copy(icon, output.join("flow.ico")).expect("Copy the original Flow icon");
    let rendered_manifest = fs::read_to_string(manifest)
        .expect("Read application manifest")
        .replace("@VERSION_QUAD@", &numbers.join("."));
    fs::write(output.join("flow.manifest"), rendered_manifest).expect("Stage application manifest");
    let rendered_rc = fs::read_to_string(rc)
        .expect("Read resource template")
        .replace("@VERSION_NUMBERS@", &numbers.join(","))
        .replace("@VERSION@", &version);
    fs::write(output.join("flow.rc"), rendered_rc).expect("Stage Windows resource script");
    let compiler = env::var_os("FLOW_WINDRES")
        .map(PathBuf::from)
        .or_else(|| {
            ["CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER", "RUSTC_LINKER"]
                .into_iter()
                .filter_map(env::var_os)
                .map(PathBuf::from)
                .filter_map(|linker| linker.parent().map(|parent| parent.join("windres.exe")))
                .find(|path| path.is_file())
        })
        .unwrap_or_else(|| PathBuf::from("windres"));
    let object = output.join("flow-resources.o");
    let result = Command::new(&compiler)
        .current_dir(&output)
        .args(["--target=pe-x86-64", "--codepage=65001", "--input-format=rc", "--output-format=coff", "--input=flow.rc"])
        .arg("--output").arg(&object)
        .env("TEMP", &output).env("TMP", &output).env("TMPDIR", &output)
        .output()
        .unwrap_or_else(|error| panic!("Cannot run {}: {error}. Install/configure the existing GNU build toolchain or set FLOW_WINDRES; this script never installs tools.", compiler.display()));
    if !result.status.success() {
        panic!(
            "Windows resource compilation failed: {}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }
    println!("cargo:rustc-link-arg-bin=flow-desktop={}", object.display());
}
