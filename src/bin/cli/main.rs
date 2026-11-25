//! `ximod-cli`, the console executable (D-06, decision D-91): the same
//! library as the window, its operations as commands.
//!
//! Under Linux and macOS the package ships a link `ximod-cli` →
//! `ximod-translator` instead (D-104); this binary serves the tests. Under
//! Windows it is a small console launcher (D-104): a window executable has
//! no console to write in, a console one opens a black window when the
//! interface starts; so `ximod-cli.exe` starts `ximod-translator.exe` as
//! the command line, in its own console, waits for it and returns its code
//! (0 / 1 / 2) — the language models and the rest are not shipped twice.

#[cfg(not(target_os = "windows"))]
fn main() {
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let stdout = std::io::stdout();
    let stderr = std::io::stderr();
    let code = ximod_translator::cli::run(args, &mut stdout.lock(), &mut stderr.lock());
    std::process::exit(code);
}

#[cfg(target_os = "windows")]
fn main() {
    use std::process::{Command, exit};
    // Ctrl+C reaches the program started too (same console): the launcher
    // lets it stop and waits for its code. A handler of its own, not
    // `SetConsoleCtrlHandler(None, TRUE)`: that « ignore Ctrl+C » is
    // inherited by the child, which then could not be stopped (recette
    // Windows du 22 novembre 2026).
    unsafe extern "system" fn wait_for_the_child(_ctrl_type: u32) -> i32 {
        1
    }
    unsafe {
        windows_sys::Win32::System::Console::SetConsoleCtrlHandler(Some(wait_for_the_child), 1);
    }
    let exe = match std::env::current_exe() {
        Ok(p) => p.with_file_name("ximod-translator.exe"),
        Err(e) => {
            eprintln!("ximod-cli: {e}");
            exit(2);
        }
    };
    let status = Command::new(&exe)
        // `ximod_translator::cli::AS_CLI_FLAG`, written out: the launcher
        // links nothing of the library and stays small.
        .arg("--ximod-cli")
        .args(std::env::args_os().skip(1))
        .status();
    match status {
        Ok(s) => exit(s.code().unwrap_or(2)),
        Err(e) => {
            eprintln!("{}: {e}", exe.display());
            exit(2);
        }
    }
}
