//! Domain model and analyses for pyscythe.
//!
//! This crate knows nothing about parsers, filesystems, or the ty database.
//! Analyses are written against the [`index::CodebaseIndex`] port so they can be
//! driven by a real adapter in production and by an in-memory fake in tests.

pub mod baseline;
pub mod boundaries;
pub mod config;
pub mod cycles;
pub mod dead_code;
pub mod deps;
pub mod dupes;
pub mod edit;
pub mod finding;
pub mod fix;
pub mod graph;
pub mod health;
pub mod index;
pub mod keep;
pub mod manifest;
pub mod metrics;
pub mod plugins;
pub mod report;
pub mod source;
pub mod symbol;
pub mod tokens;

#[cfg(test)]
pub(crate) mod testing;
