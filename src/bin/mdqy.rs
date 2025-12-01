//! The `mdqy` binary. Delegates to the library's [`mdqy::run_cli`].

fn main() {
    match mdqy::run_cli() {
        Ok(()) => {}
        Err(e) => {
            eprintln!("mdqy: {e}");
            std::process::exit(4);
        }
    }
}
