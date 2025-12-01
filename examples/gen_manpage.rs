//! Emit a roff man page to stdout.
//!
//! ```sh
//! cargo run --example gen_manpage > mdqy.1
//! ```

use std::io;

fn main() {
    let cmd = mdqy::cli_command();
    let man = clap_mangen::Man::new(cmd);
    man.render(&mut io::stdout()).expect("render");
}
