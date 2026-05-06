// build.rs — Parsec Web v1.3
// Pre-compiles GPU shaders and generates the shader cache key.
//
// macOS: metal/neutron.metal → metal/neutron.metallib via xcrun
//        Loaded at runtime via MTLNewLibraryWithData — zero compile time.
// All:   Generates PARSEC_SHADER_CACHE_KEY env var for JIT cache invalidation.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let metal_src = manifest_dir.join("../metal/neutron.metal");
    let metal_air = manifest_dir.join("../metal/neutron.air");
    let metal_lib = manifest_dir.join("../metal/neutron.metallib");

    println!("cargo:rerun-if-changed=../metal/neutron.metal");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../gpui/renderer/neutron_bridge.rs");

    // macOS: compile Metal shader to .metallib
    if cfg!(target_os = "macos") {
        compile_metal_shader(&metal_src, &metal_air, &metal_lib);
        println!("cargo:rustc-link-lib=framework=Metal");
        println!("cargo:rustc-link-lib=framework=QuartzCore");
        println!("cargo:rustc-link-lib=framework=Cocoa");
    }

    // All platforms: shader cache invalidation key
    generate_shader_cache_key(&manifest_dir);
}

fn compile_metal_shader(src: &PathBuf, air: &PathBuf, lib: &PathBuf) {
    if !src.exists() {
        println!("cargo:warning=metal/neutron.metal not found — Metal shader compiled at runtime");
        return;
    }
    // Check xcrun available (requires Xcode command line tools)
    if Command::new("xcrun").arg("--find").arg("metal").output().is_err() {
        println!("cargo:warning=xcrun not found — install Xcode CLI tools for pre-compiled shaders");
        return;
    }
    // Compile .metal → .air
    let s1 = Command::new("xcrun").args([
        "metal", "-c", "-O3", "-std=metal3.0", "-ffast-math",
        "-mmacosx-version-min=12.0",
        src.to_str().unwrap(), "-o", air.to_str().unwrap(),
    ]).status();
    match s1 {
        Ok(s) if s.success() => {}
        Ok(s) => { println!("cargo:warning=metal compile failed ({s}) — runtime fallback"); return; }
        Err(e) => { println!("cargo:warning=xcrun metal error: {e}"); return; }
    }
    // Link .air → .metallib
    let s2 = Command::new("xcrun").args([
        "metallib", air.to_str().unwrap(), "-o", lib.to_str().unwrap(),
    ]).status();
    match s2 {
        Ok(s) if s.success() => println!("cargo:warning=Neutron: pre-compiled Metal shader ready"),
        Ok(s) => println!("cargo:warning=metallib failed ({s})"),
        Err(e) => println!("cargo:warning=metallib error: {e}"),
    }
    let _ = std::fs::remove_file(air);
}

fn generate_shader_cache_key(manifest_dir: &PathBuf) {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    for f in &["../metal/neutron.metal", "../gpui/renderer/neutron_bridge.rs"] {
        if let Ok(b) = std::fs::read(manifest_dir.join(f)) { b.hash(&mut h); }
    }
    let key = h.finish();
    let _ = std::fs::write(manifest_dir.join("../shader_cache_key.txt"), format!("{key:016x}"));
    println!("cargo:rustc-env=PARSEC_SHADER_CACHE_KEY={key:016x}");
}
