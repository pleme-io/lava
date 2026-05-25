//! Library surface for the `lava` CLI. Exposes the typed cli module
//! so integration tests can drive the command-dispatcher with
//! captured writers (no subprocess spawning).

#![allow(clippy::module_name_repetitions)]

pub mod cli;
