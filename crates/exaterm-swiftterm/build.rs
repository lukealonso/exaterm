use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let swift_src = manifest_dir.join("swift/Sources/ExatermTerminalBridge/Bridge.swift");
    let swiftterm_src = manifest_dir.join("vendor/SwiftTerm/Sources/SwiftTerm");
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let lib_file = out_dir.join("libExatermTerminalBridge.a");
    let objc_header = out_dir.join("Bridge-Swift.h");
    let module_cache = out_dir.join("clang-module-cache");
    fs::create_dir_all(&module_cache).expect("failed to create swift module cache");
    let swiftc = swiftc_path();
    let sdk = macos_sdk_path();

    let mut swift_sources = Vec::new();
    collect_swift_sources(&swiftterm_src, &mut swift_sources);
    swift_sources.push(swift_src.clone());

    let output = Command::new(&swiftc)
        .current_dir(&out_dir)
        .arg("-emit-library")
        .arg("-static")
        .arg("-o")
        .arg(&lib_file)
        .arg("-sdk")
        .arg(&sdk)
        .arg("-module-cache-path")
        .arg(&module_cache)
        .arg("-module-name")
        .arg("ExatermTerminalBridge")
        .arg("-emit-objc-header-path")
        .arg(&objc_header)
        .arg("-O")
        .arg("-whole-module-optimization")
        .arg("-parse-as-library")
        .env("CLANG_MODULE_CACHE_PATH", &module_cache)
        .args(&swift_sources)
        .output()
        .expect("failed to invoke swiftc — is Xcode installed?");

    if !output.status.success() {
        panic!(
            "swiftc compilation failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }

    // Tell cargo to link against our static library.
    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=ExatermTerminalBridge");
    println!(
        "cargo:rustc-link-arg=-Wl,-force_load,{}",
        lib_file.display()
    );
    println!(
        "cargo:rustc-link-search=native={}",
        sdk.join("usr/lib/swift").display()
    );
    println!(
        "cargo:rustc-link-search=native={}",
        swiftc
            .parent()
            .and_then(|path| path.parent())
            .map(|path| path.join("lib/swift-5.0/macosx"))
            .expect("swift toolchain layout changed")
            .display()
    );

    // Link the Swift runtime and required system frameworks.
    // Find the Swift runtime library path from the toolchain.
    let swift_lib_output = Command::new(&swiftc)
        .args(["-print-target-info"])
        .arg("-sdk")
        .arg(&sdk)
        .env("CLANG_MODULE_CACHE_PATH", &module_cache)
        .output()
        .expect("failed to query swift target info");

    if !swift_lib_output.status.success() {
        panic!("swiftc -print-target-info failed");
    }

    let info = String::from_utf8_lossy(&swift_lib_output.stdout);

    // Parse the runtime library path from the JSON output.
    if let Some(paths) = extract_swift_lib_paths(&info) {
        for path in paths {
            println!("cargo:rustc-link-search=native={path}");
        }
    }

    // Link Swift runtime and system frameworks.
    println!("cargo:rustc-link-lib=dylib=swiftCore");
    println!("cargo:rustc-link-lib=dylib=swiftFoundation");
    println!("cargo:rustc-link-lib=dylib=swiftAppKit");
    println!("cargo:rustc-link-lib=dylib=swiftObjectiveC");
    println!("cargo:rustc-link-lib=dylib=swiftCompression");
    println!("cargo:rustc-link-lib=dylib=swiftCoreFoundation");
    println!("cargo:rustc-link-lib=dylib=swiftCoreGraphics");
    println!("cargo:rustc-link-lib=dylib=swiftCoreImage");
    println!("cargo:rustc-link-lib=dylib=swiftDispatch");
    println!("cargo:rustc-link-lib=dylib=swiftIOKit");
    println!("cargo:rustc-link-lib=dylib=swiftMetal");
    println!("cargo:rustc-link-lib=dylib=swiftMetalKit");
    println!("cargo:rustc-link-lib=dylib=swiftModelIO");
    println!("cargo:rustc-link-lib=dylib=swiftOSLog");
    println!("cargo:rustc-link-lib=dylib=swiftQuartzCore");
    println!("cargo:rustc-link-lib=dylib=swiftSpatial");
    println!("cargo:rustc-link-lib=dylib=swiftUniformTypeIdentifiers");
    println!("cargo:rustc-link-lib=dylib=swiftXPC");
    println!("cargo:rustc-link-lib=dylib=swiftsimd");
    println!("cargo:rustc-link-lib=dylib=swiftos");
    println!("cargo:rustc-link-lib=framework=AppKit");
    println!("cargo:rustc-link-lib=framework=Foundation");
    println!("cargo:rustc-link-lib=framework=Carbon");
    println!("cargo:rustc-link-lib=framework=CoreGraphics");
    println!("cargo:rustc-link-lib=framework=CoreText");
    println!("cargo:rustc-link-lib=framework=ImageIO");
    println!("cargo:rustc-link-lib=framework=Metal");
    println!("cargo:rustc-link-lib=framework=MetalKit");
    println!("cargo:rustc-link-lib=framework=QuartzCore");
    println!("cargo:rustc-link-lib=framework=SwiftUI");
    println!("cargo:rustc-link-lib=framework=UniformTypeIdentifiers");

    // Rerun if the Swift source changes.
    println!("cargo:rerun-if-changed=swift/Sources/ExatermTerminalBridge/Bridge.swift");
    println!("cargo:rerun-if-changed=vendor/SwiftTerm/Sources/SwiftTerm");
}

fn collect_swift_sources(dir: &PathBuf, output: &mut Vec<PathBuf>) {
    let mut entries = fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", dir.display()))
        .map(|entry| entry.expect("failed to read dir entry").path())
        .collect::<Vec<_>>();
    entries.sort();

    for path in entries {
        if path.is_dir() {
            collect_swift_sources(&path, output);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) == Some("swift") {
            output.push(path);
        }
    }
}

fn swiftc_path() -> PathBuf {
    let xcode_swiftc = PathBuf::from(
        "/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/bin/swiftc",
    );
    if xcode_swiftc.exists() {
        return xcode_swiftc;
    }

    PathBuf::from(env::var("SWIFTC").unwrap_or_else(|_| "swiftc".to_string()))
}

fn macos_sdk_path() -> PathBuf {
    let xcode_sdk = PathBuf::from(
        "/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk",
    );
    if xcode_sdk.exists() {
        return xcode_sdk;
    }

    PathBuf::from("/Library/Developer/CommandLineTools/SDKs/MacOSX.sdk")
}

/// Extract Swift runtime library search paths from `swiftc -print-target-info` JSON output.
fn extract_swift_lib_paths(info: &str) -> Option<Vec<String>> {
    // The output is JSON with "runtimeLibraryPaths" array.
    let mut paths = Vec::new();
    let mut in_runtime_paths = false;
    for line in info.lines() {
        let trimmed = line.trim();
        if trimmed.contains("runtimeLibraryPaths") {
            in_runtime_paths = true;
            continue;
        }
        if in_runtime_paths && trimmed.starts_with(']') {
            break;
        }
        if in_runtime_paths {
            let path = trimmed.trim_matches(|c| c == '"' || c == ',');
            if !path.is_empty() {
                paths.push(path.to_string());
            }
        }
    }
    if paths.is_empty() { None } else { Some(paths) }
}
