//! lava — operator CLI for the lava suite.
//!
//! ```text
//! lava plan <PATH>     [--binding k=v] [--gate <iface>] [--out <FILE>] [--format json|yaml]
//! lava render <NAME>   [--binding k=v] [--out <FILE>] [--format json|yaml]
//! lava validate <PATH> [--binding k=v]  --gate <iface>
//! lava ls architectures
//! lava ls interfaces
//! lava show interface <NAME>
//! ```

#![allow(clippy::module_name_repetitions)]

fn main() {
    let exit = lava::cli::run(std::env::args_os());
    std::process::exit(exit);
}
