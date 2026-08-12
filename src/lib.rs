//! `loxahatchee_river_fl` — the library behind the `lrfl` CLI.
//!
//! Business logic (the WIPP API client, account models, auth session, output
//! formatting) lives here so it is unit-testable and reusable; `main.rs` is a
//! thin binary that parses arguments and dispatches into [`commands`].

pub mod account;
pub mod acct;
pub mod auth;
pub mod bill;
pub mod bills;
pub mod cli;
pub mod client;
pub mod commands;
pub mod config;
pub mod error;
pub mod formatter;
pub mod model;
pub mod update;
pub mod util;
pub mod version;
