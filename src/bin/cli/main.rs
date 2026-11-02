//! `ximod-cli`, the console executable (D-06, decision D-91): the same
//! library as the window, its operations as commands. Everything is in
//! [`ximod_translator::cli`]; this only hands it the arguments and the
//! standard outputs, and returns its code (0 / 1 / 2).

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
