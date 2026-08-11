//! Thin binary: parse arguments and dispatch into the library's `commands`.
//! All logic lives in the `loxahatchee_river_fl` crate (see `src/lib.rs`).

use clap::Parser;

use loxahatchee_river_fl::cli::{Cli, Command};
use loxahatchee_river_fl::commands::{self, Ctx};
use loxahatchee_river_fl::error::AppError;

fn run(cli: Cli) -> Result<(), AppError> {
    // `completions` only needs the clap command definition, not a client/session.
    if let Command::Completions { shell } = &cli.command {
        return commands::completions::run(*shell);
    }

    let ctx = Ctx::new(&cli)?;
    match &cli.command {
        Command::Summary(a) => commands::summary::run(&ctx, a),
        Command::Account(a) => commands::account::show(&ctx, a),
        Command::Balance(a) => commands::account::balance(&ctx, a),
        Command::Charges(a) => commands::account::charges(&ctx, a),
        Command::Status(a) => commands::status::run(&ctx, a),
        Command::History {
            account,
            since,
            years,
            limit,
        } => commands::history::run(&ctx, account, since.as_deref(), *years, *limit),
        Command::Pay { account, open } => commands::pay::pay(&ctx, account, *open),
        Command::Open(a) => commands::pay::open(&ctx, a),
        Command::Bill {
            account,
            open,
            save,
        } => commands::bill::run(&ctx, account, *open, save.as_deref()),
        Command::Search {
            query,
            limit,
            balances,
            full,
        } => commands::search::run(&ctx, query, *limit, *balances, *full),
        Command::District => commands::district::run(&ctx),
        Command::Config { action } => commands::config::run(&ctx, action),
        Command::Auth(cmd) => commands::auth::run(&ctx, cmd),
        Command::Login => commands::auth::login(&ctx),
        Command::Logout => commands::auth::logout(&ctx),
        Command::Whoami => commands::auth::whoami(&ctx),
        Command::Info => commands::auth::info(&ctx),
        Command::Accounts { balances } => commands::accounts::run(&ctx, *balances),
        Command::SelfUpdate(args) => commands::self_update::run(&ctx, args),
        Command::Completions { .. } => unreachable!("handled above"),
    }
}

fn main() {
    let cli = Cli::parse();
    let json_mode = cli.json;
    if let Err(e) = run(cli) {
        std::process::exit(pk_cli_core::output::fail(&e, json_mode));
    }
}
