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

    // Require node_modules to exist.  Run `npm install` in the frontend/
    // directory (or `make web`) before building.  Keeping network fetches out
    // of `cargo build` makes the build reproducible and offline-safe.
    //
    // When node_modules is missing we emit a warning and create placeholder
    // files so that `cargo check --workspace` still succeeds (the embedded
    // dist will contain stubs, but the binary won't be usable).
    let node_modules = frontend_dir.join("node_modules");
    if !node_modules.exists() {
        println!(
            "cargo:warning=frontend/node_modules not found — skipping frontend build. \
             Run `npm install` in crates/exaterm-web/frontend/ (or `make web`) to build \
             the full web UI."
        );
        ensure_placeholder_dist(&dist_dir);
        return;
    }

    // Bundle TypeScript + CSS with esbuild.
    // outdir is relative to frontend_dir since we set current_dir there.
    let output = Command::new("npx")
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
        .output()
        .expect("failed to run esbuild (is node installed?)");
    if !output.status.success() {
        eprintln!("{}", String::from_utf8_lossy(&output.stderr));
        panic!("esbuild bundling failed");
    }

    // Copy static assets to dist/ (recursive).
    let static_dir = frontend_dir.join("static");
    if static_dir.is_dir() {
        copy_dir_recursive(&static_dir, &dist_dir);
    }
}

/// Create minimal placeholder files so the `include_dir!` macro and the
/// embedded-asset tests have something to include when the frontend hasn't
/// been built.
fn ensure_placeholder_dist(dist_dir: &Path) {
    for (name, content) in [
        ("index.html", "<!doctype html><html><head>\
            <link rel=\"stylesheet\" href=\"/assets/app.css\">\
            <link rel=\"stylesheet\" href=\"/assets/main.css\">\
            </head><body>frontend not built\
            <script src=\"/assets/main.js\"></script>\
            </body></html>"),
        ("main.js", "// placeholder"),
        ("main.css", "/* placeholder */"),
        ("app.css", "/* placeholder */"),
    ] {
        let path = dist_dir.join(name);
        if !path.exists() {
            fs::write(&path, content).expect("failed to write placeholder");
        }
    }
}

fn copy_dir_recursive(src: &Path, dst: &Path) {
    for entry in fs::read_dir(src).expect("failed to read directory") {
        let entry = entry.expect("failed to read dir entry");
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            fs::create_dir_all(&dst_path).expect("failed to create directory");
            copy_dir_recursive(&src_path, &dst_path);
        } else {
            fs::copy(&src_path, &dst_path).expect("failed to copy static asset");
        }
    }
}
