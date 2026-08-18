use clap::{Parser, Subcommand};
use clap_complete::Shell;

/// View account, billing, and payment information for Loxahatchee River District
/// utilities from the command line.
///
/// Reads are the same anonymous guest-view lookups the portal makes before you
/// log in — no account or password required. You identify an account by its
/// number (`NNNNNNN-N`); set a default once with `lrfl config set-account` and
/// most commands need no argument.
#[derive(Parser, Debug)]
#[command(name = "lrfl", version, about, long_about = None)]
pub struct Cli {
    /// Emit machine-readable JSON on stdout (diagnostics go to stderr).
    #[arg(long, global = true)]
    pub json: bool,

    /// Extra diagnostics on stderr (never sensitive data).
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Suppress non-error stderr output.
    #[arg(short, long, global = true)]
    pub quiet: bool,

    /// Disable ANSI color (reserved; output is currently plain).
    #[arg(long, global = true)]
    pub no_color: bool,

    /// WIPP tenant id. Defaults to LOXA (Loxahatchee River District).
    #[arg(long, global = true, env = "LRFL_WIPP_ID", default_value_t = crate::client::DEFAULT_WIPP_ID.to_string())]
    pub wipp_id: String,

    /// Login email for authenticated commands. Falls back to $LRFL_EMAIL, then
    /// the email saved by `lrfl login`.
    #[arg(long, global = true, env = "LRFL_EMAIL")]
    pub email: Option<String>,

    #[command(subcommand)]
    pub command: Command,
}

/// The account-number argument shared by most commands: an optional positional
/// that falls back to `$LRFL_ACCOUNT`, then the saved default account.
#[derive(clap::Args, Debug)]
pub struct AccountArg {
    /// Utility account number, `NNNNNNN-N` (e.g. 1234567-0). Falls back to
    /// $LRFL_ACCOUNT, then the default set via `lrfl config set-account`.
    #[arg(value_name = "ACCOUNT", env = "LRFL_ACCOUNT")]
    pub account: Option<String>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// One-shot overview: balance, per-service status, and last payment.
    Summary(AccountArg),

    /// Show the full account record: owner, service location, and balance.
    Account(AccountArg),

    /// Show just the amount due (per service and total).
    Balance(AccountArg),

    /// Show detailed per-service charges, meter readings, and usage.
    Charges(AccountArg),

    /// Show each service's active/inactive status (an async portal lookup).
    Status(AccountArg),

    /// List recent payments posted to the account.
    History {
        #[command(flatten)]
        account: AccountArg,

        /// Only payments on or after this ISO date (YYYY-MM-DD). Overrides --years.
        #[arg(long, value_name = "YYYY-MM-DD")]
        since: Option<String>,

        /// Look back this many years (ignored if --since is given).
        #[arg(long, default_value_t = 3)]
        years: u32,

        /// Only show the most recent N payments.
        #[arg(long, value_name = "N")]
        limit: Option<usize>,
    },

    /// Compute the amount due and hand off to the official portal to pay.
    ///
    /// Card capture runs through the district's payment gateway (BluePay/FIS)
    /// behind a reCAPTCHA, so this prints — or with `--open`, launches — the
    /// portal's secure "Pay Now" page for the account rather than handling a
    /// card itself.
    Pay {
        #[command(flatten)]
        account: AccountArg,

        /// Open the payment page in your default browser.
        #[arg(long)]
        open: bool,
    },

    /// Open the account's page in the portal in your default browser.
    Open(AccountArg),

    /// Show the current bill parsed from the official PDF: bill-to owner, mailing
    /// address, AutoPay status, service period, last payment, total due — data the
    /// API redacts/omits. `--open` opens the PDF; `--save PATH` downloads it.
    ///
    /// For prior periods, see `lrfl bills list` / `lrfl bills get`.
    Bill {
        #[command(flatten)]
        account: AccountArg,
        /// Open the PDF bill in your browser instead of parsing it.
        #[arg(long)]
        open: bool,
        /// Download the PDF bill to this file instead of parsing it.
        #[arg(long, value_name = "PATH")]
        save: Option<String>,
    },

    /// Historical bills: list past periods (`bills list`) and download any
    /// period's official PDF by id (`bills get <YYYY-MM-DD>`), matching the
    /// piekstra-cli/1 `bills list` + `bills get <id>` shape.
    Bills {
        #[command(subcommand)]
        action: BillsCmd,
    },

    /// Statement documents: list and download official bill PDFs (documents/v1
    /// profile). A document's id is its ISO due date, so this is the same PDF
    /// surface as `bills`, under the shared profile spelling.
    Documents {
        #[command(subcommand)]
        action: DocumentsCmd,
    },

    /// Find accounts by street/property address (e.g. `lrfl search "MAPLE"`).
    /// The district matches server-side (case-insensitive substring); no login.
    /// Pass a match's account number to `bill`, `account`, or `balance` for full
    /// detail — or use `--full` here to fold that detail into the results.
    Search {
        /// Street name or address fragment to match.
        query: String,
        /// Maximum number of matches to return.
        #[arg(long, default_value_t = 50)]
        limit: u32,
        /// Also fetch each match's balance due (one request per match — use with
        /// a focused query or a small `--limit`). Matches `accounts --balances`.
        #[arg(long, short = 'b')]
        balances: bool,
        /// Enrich each match with full bill detail (owner, mailing address,
        /// AutoPay, service period, total due) parsed from its official PDF.
        /// Fetches a bill per match, so it is capped to a small result set —
        /// narrow the query or lower `--limit`. Implies `--balances`.
        #[arg(long, conflicts_with = "balances")]
        full: bool,
    },

    /// Show district info: name, billed services, payment options, contact.
    District,

    /// Manage the saved default account number (stored in plain config, not a secret).
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// Credential management: login, logout, status, whoami.
    #[command(subcommand)]
    Auth(AuthCmd),

    /// Log in with your portal email + password (stored in the OS keychain).
    ///
    /// Hidden alias for `auth login` (the canonical piekstra-cli/1 spelling),
    /// kept for back-compat.
    #[command(hide = true)]
    Login,

    /// Hidden alias for `auth logout`, kept for back-compat.
    #[command(hide = true)]
    Logout,

    /// Hidden alias for `auth whoami`, kept for back-compat.
    #[command(hide = true)]
    Whoami,

    /// List the utility accounts linked to your login. Requires login.
    Accounts {
        /// Also fetch and show the amount due on each account.
        #[arg(short, long)]
        balances: bool,
    },

    /// Update lrfl to the latest release from GitHub.
    #[command(name = "self-update")]
    SelfUpdate(pk_cli_selfupdate::SelfUpdateArgs),

    /// Machine-readable capability discovery (cli-info/v1).
    Info,

    /// Print a shell completion script (e.g. `lrfl completions zsh`).
    Completions {
        /// Shell to generate completions for.
        #[arg(value_enum)]
        shell: Shell,
    },
}

#[derive(Subcommand, Debug)]
pub enum BillsCmd {
    /// Enumerate discoverable bill periods (current + up to 3 prior per service).
    #[command(visible_alias = "ls")]
    List {
        #[command(flatten)]
        account: AccountArg,
        /// Filter to one service by name (e.g. `Sewer`, `Water`).
        #[arg(long, value_name = "NAME")]
        service: Option<String>,
    },
    /// Download a bill PDF by its period id (`YYYY-MM-DD` due date).
    /// `-o -` streams to stdout; `--all` downloads every listed period.
    ///
    /// You can also pass a due date that isn't in `bills list` (e.g. a period
    /// older than the account's 3-prior window); WIPP's hosted-PDF archive
    /// serves those on request when it has them, and this errors cleanly when
    /// it doesn't.
    #[command(visible_alias = "download")]
    Get {
        /// Period id: the due date shown by `bills list`, e.g. `2026-05-13`.
        /// Omit and pass `--all` to download every discoverable period.
        #[arg(value_name = "PERIOD-ID")]
        period_id: Option<String>,
        /// Account number, `NNNNNNN-N`. Falls back to `$LRFL_ACCOUNT`, then
        /// the default set via `lrfl config set-account`, then your logged-in
        /// linked account.
        #[arg(value_name = "ACCOUNT", env = "LRFL_ACCOUNT")]
        account: Option<String>,
        /// Download every listed period into `-o DIR` (or the current dir).
        #[arg(long, conflicts_with = "period_id")]
        all: bool,
        /// Output target. Single id: a file path or `-` for stdout. `--all`: a
        /// directory. Default: `./lrfl-bill-<account>-<YYYY-MM-DD>.pdf` in the
        /// current directory (or `<DIR>/lrfl-bill-<account>-<YYYY-MM-DD>.pdf`
        /// per file, with `--all`).
        #[arg(long, short, value_name = "PATH")]
        output: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum DocumentsCmd {
    /// List downloadable statement documents, newest first (document-list/v1).
    #[command(visible_alias = "ls")]
    List {
        #[command(flatten)]
        account: AccountArg,
        /// Filter to one service by name (e.g. `Sewer`, `Water`).
        #[arg(long, value_name = "NAME")]
        service: Option<String>,
    },
    /// Download a statement PDF by id (its ISO due date), or every one with
    /// `--all`. `-o -` streams a single document to stdout.
    #[command(visible_alias = "get")]
    Download {
        /// Document id from `documents list` — the ISO due date, e.g.
        /// `2026-05-13`. Omit and pass `--all` to download every one.
        #[arg(value_name = "ID")]
        id: Option<String>,
        /// Account number, `NNNNNNN-N` (falls back to `$LRFL_ACCOUNT`, config,
        /// or your linked account).
        #[arg(value_name = "ACCOUNT", env = "LRFL_ACCOUNT")]
        account: Option<String>,
        /// Download every statement into `-o DIR` (or the current dir).
        #[arg(long, conflicts_with = "id")]
        all: bool,
        /// Output target: a file or `-` for stdout (single id), or a directory
        /// (with `--all`). Default: `./lrfl-bill-<account>-<id>.pdf`.
        #[arg(long, short, value_name = "PATH")]
        output: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum AuthCmd {
    /// Log in with your portal email + password (stored in the OS keychain).
    Login,
    /// Log out: remove the stored credential from the keychain.
    Logout,
    /// Credential and session status (auth-status/v1 with --json).
    Status,
    /// Show who you're logged in as (identity from the session token).
    Whoami,
}

#[derive(Subcommand, Debug)]
pub enum ConfigAction {
    /// Save a default account so commands can be run without an ACCOUNT argument.
    SetAccount {
        /// Utility account number, `NNNNNNN-N`.
        account: String,
    },
    /// Forget the saved default account.
    Clear,
    /// Show the current default account and where it's stored.
    Show,
}
