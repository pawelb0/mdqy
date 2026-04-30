use std::io;

use clap_complete::{generate, Shell};

fn main() {
    let shell = std::env::args()
        .nth(1)
        .and_then(|s| s.parse::<Shell>().ok())
        .unwrap_or_else(|| {
            eprintln!("usage: gen_completions <bash|zsh|fish|elvish|powershell>");
            std::process::exit(2);
        });
    let mut cmd = mdqy::cli_command();
    let name = cmd.get_name().to_string();
    generate(shell, &mut cmd, name, &mut io::stdout());
}
