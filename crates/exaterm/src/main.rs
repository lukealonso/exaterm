use std::env;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

fn main() {
    let current_exe = env::current_exe().expect("failed to resolve current executable");
    let exe_dir = current_exe
        .parent()
        .expect("executable has no parent directory");

    let gtk_bin = exe_dir.join("exaterm-gtk");
    if !gtk_bin.exists() {
        eprintln!(
            "exaterm: could not find exaterm-gtk at {}",
            gtk_bin.display()
        );
        std::process::exit(1);
    }

    let args: Vec<String> = env::args().skip(1).collect();

    // Double-fork: the child setsid's and execs exaterm-gtk, fully detached
    // from the launching terminal. The parent exits immediately.
    unsafe {
        let pid = libc::fork();
        if pid < 0 {
            eprintln!("exaterm: fork failed");
            std::process::exit(1);
        }
        if pid > 0 {
            // Parent — exit immediately so the shell gets its prompt back.
            std::process::exit(0);
        }
        // Child — new session so we're detached from the controlling terminal.
        libc::setsid();
    }

    let err = Command::new(&gtk_bin)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .exec();

    eprintln!("exaterm: failed to exec exaterm-gtk: {err}");
    std::process::exit(1);
}
