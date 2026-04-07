use std::fs;
use std::path::Path;
use std::process::Command;

fn main() {
    let frontend_dir = Path::new("frontend");
    let dist_dir = frontend_dir.join("dist");

    println!("cargo:rerun-if-changed=frontend/src");
    println!("cargo:rerun-if-changed=frontend/static");
    println!("cargo:rerun-if-changed=frontend/package.json");
    println!("cargo:rerun-if-changed=frontend/package-lock.json");
    println!("cargo:rerun-if-changed=frontend/tsconfig.json");

    fs::create_dir_all(&dist_dir).expect("failed to create frontend/dist");

    // Install npm dependencies if node_modules is missing.
    let node_modules = frontend_dir.join("node_modules");
    if !node_modules.exists() {
        let status = Command::new("npm")
            .arg("install")
            .current_dir(frontend_dir)
            .status()
            .expect("failed to run npm install (is node installed?)");
        assert!(status.success(), "npm install failed");
    }

    // Bundle TypeScript + CSS with esbuild.
    // outdir is relative to frontend_dir since we set current_dir there.
    let status = Command::new("npx")
        .args([
            "esbuild",
            "src/main.ts",
            "--bundle",
            "--outdir=dist",
            "--entry-names=[name]",
            "--minify",
            "--sourcemap",
        ])
        .current_dir(frontend_dir)
        .status()
        .expect("failed to run esbuild (is node installed?)");
    assert!(status.success(), "esbuild bundling failed");

    // Copy static assets to dist/.
    let static_dir = frontend_dir.join("static");
    if static_dir.is_dir() {
        for entry in fs::read_dir(&static_dir).expect("failed to read frontend/static") {
            let entry = entry.expect("failed to read static dir entry");
            let dest = dist_dir.join(entry.file_name());
            fs::copy(entry.path(), dest).expect("failed to copy static asset");
        }
    }
}
