//! Command implementations — one thin module per domain. Each function takes a
//! shared [`Ctx`] plus its parsed arguments, calls the client, and hands the
//! result to the [`formatter`](crate::formatter). `main.rs` only wires args to
//! these.

pub mod account;
pub mod accounts;
pub mod auth;
pub mod bill;
pub mod bills;
pub mod completions;
pub mod config;
pub mod district;
pub mod history;
pub mod pay;
pub mod search;
pub mod self_update;
pub mod status;
pub mod summary;

use crate::acct::AccountId;
use crate::auth::Session;
use crate::cli::{AccountArg, Cli};
use crate::client::Wipp;
use crate::error::AppError;
use crate::model::LinkedAccount;

/// Per-invocation context: the API client, the login session, and the global
/// flags every command reads. Built once in `main` and passed to each command.
pub struct Ctx {
    pub api: Wipp,
    pub session: Session,
    pub json: bool,
    pub verbose: bool,
    pub quiet: bool,
    pub login: Option<String>,
}

impl Ctx {
    pub fn new(cli: &Cli) -> Result<Self, AppError> {
        Ok(Ctx {
            api: Wipp::new(cli.wipp_id.clone())?,
            session: Session::new(),
            json: cli.json,
            verbose: cli.verbose,
            quiet: cli.quiet,
            login: cli.email.clone(),
        })
    }

    /// Emit a diagnostic line (stderr) when `--verbose` and not `--quiet`.
    pub fn log(&self, msg: &str) {
        if self.verbose && !self.quiet {
            eprintln!("{msg}");
        }
    }

    /// Resolve the login name and mint a fresh `id_token` for an authenticated
    /// command. Returns `(login, token)`.
    pub fn authed(&self) -> Result<(String, String), AppError> {
        let login = self.session.resolve_login(self.login.as_deref())?;
        let token = self.session.access_token(&self.api, &login)?;
        Ok((login, token))
    }

    /// Resolve an account id: the positional arg / `$LRFL_ACCOUNT`, then the saved
    /// default, and finally — if you're logged in and gave nothing else — the
    /// single utility account linked to your login.
    pub fn resolve_account(&self, arg: &AccountArg) -> Result<AccountId, AppError> {
        if let Some(raw) = arg
            .account
            .clone()
            .filter(|s| !s.trim().is_empty())
            .or_else(crate::config::load_default_account)
        {
            return AccountId::parse(&raw);
        }
        // Only try the login fallback when a session looks configured, so guest
        // use never triggers an unexpected auth prompt.
        let logged_in = self.login.is_some() || crate::config::load_session_email().is_some();
        if logged_in {
            match self.account_from_login() {
                Ok(id) => {
                    self.log(&format!("using your linked account {}", id.dashed()));
                    return Ok(id);
                }
                // A "specify which" error is worth surfacing; other failures fall
                // through to the generic hint below.
                Err(e @ AppError::Usage(_)) => return Err(e),
                Err(_) => {}
            }
        }
        Err(AppError::Usage(
            "no account — pass one as NNNNNNN-N, set $LRFL_ACCOUNT, run \
             `lrfl config set-account <id>`, or `lrfl login`"
                .into(),
        ))
    }

    /// The single utility account linked to the login, if there's exactly one.
    fn account_from_login(&self) -> Result<AccountId, AppError> {
        let (_, token) = self.authed()?;
        let utils: Vec<LinkedAccount> =
            LinkedAccount::list_from(&self.api.billing_accounts(&token)?)
                .into_iter()
                .filter(LinkedAccount::is_utility)
                .collect();
        match utils.as_slice() {
            [one] => AccountId::parse(&one.account_id),
            [] => Err(AppError::NotFound(
                "no utility accounts linked to your login".into(),
            )),
            many => Err(AppError::Usage(format!(
                "{} utility accounts linked to your login — specify which (see `lrfl accounts`)",
                many.len()
            ))),
        }
    }
}
