//! `lrfl mcp`: the shared `utility/v1` tools (`pk_cli_utility::mcp`) over the
//! district's WIPP API.
//!
//! This module supplies what is the district's own: [`Portal`], the reads the
//! commands already make, and [`LoxSource`], which maps them with the commands'
//! mappers in `formatter.rs`. Tests put fixtures behind [`Portal`], so nothing
//! here needs a network or a login to test.
//!
//! An account's reads are anonymous, by account number. A login adds only the
//! list of linked accounts, and a server with a default account or no login
//! configured never touches the keychain. It reads a credential and never writes
//! one.
//!
//! The commands' mappers are best-effort, for a person who can see the raw
//! payload. A tool's answer goes to a model as fact, so here an answer the
//! district did not give (a balance no field states, rows none of which can be
//! read) is an upstream error, never 0.00 or an empty list.

use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use pk_cli_mcp::Server;
use pk_cli_utility::mcp::{UtilityAccount, UtilitySource, UtilityTools};
use pk_cli_utility::{Payment as PaymentDto, Statement, UsagePeriod, UtilitySummary};
use serde_json::Value;

use crate::account::{self, Account};
use crate::acct::AccountId;
use crate::auth::{session, Secret, Session};
use crate::bills::{self, BillPeriod};
use crate::client::Wipp;
use crate::config;
use crate::error::AppError;
use crate::formatter;
use crate::model::{LinkedAccount, Payment};
use crate::util;

/// The default account's environment variable, which a Claude Desktop
/// extension's settings fill in.
const ACCOUNT_ENV: &str = "LRFL_ACCOUNT";

/// How far back payments go when a call names no `since`, as `lrfl history`.
const PAYMENT_YEARS: u32 = 3;

/// How long the login's list of accounts is kept before signing in again.
const KEEP_LINKED: Duration = Duration::from_secs(60 * 60);

/// The tools `lrfl mcp` serves. Nothing here reads a file: listing the tools
/// touches none, and a server left running sees what the user sets next.
pub fn tools(
    wipp_id: String,
    login: Option<String>,
) -> Result<UtilityTools<LoxSource<LivePortal>>, AppError> {
    let source = LoxSource {
        portal: LivePortal {
            api: Wipp::new(wipp_id)?,
            session: Session::new(),
            login: login.filter(|l| !l.trim().is_empty()),
            linked: Mutex::default(),
        },
    };
    Ok(with_source(
        source,
        std::env::var_os(session::ENV_PASSWORD).is_some(),
    ))
}

fn with_source<P: Portal>(
    source: LoxSource<P>,
    password_in_env: bool,
) -> UtilityTools<LoxSource<P>> {
    let mut server = Server::new(
        "lrfl",
        env!("CARGO_PKG_VERSION"),
        "Loxahatchee River District",
        "loxahatchee",
    )
    .instructions(
        "Loxahatchee River District access here is read-only: no tool can make a payment or \
         change anything. An account is read by its number, NNNNNNN-N; the district's lookup \
         needs no sign-in, and a sign-in only adds the list of linked accounts. Bills are the \
         current period and up to three before it, which is all the district reports here. \
         Usage exists only for metered services, in the district's own meter units, so an \
         unmetered account has none.",
    );
    // The keychain is not read while the variable is set, so `lrfl auth login`
    // would store a password this server never sees.
    if password_in_env {
        server = server.hosted(
            "through the open-source `lrfl` tool, with a password from the settings of the \
             program that started this server; it stays on the user's machine",
            "ask the user to update $LRFL_PASSWORD where it is set, then restart this server",
        );
    }
    UtilityTools::new(server, source).metering("metered service use")
}

/// The district reads the tools need. The JSON is the district's own, as the
/// models parse it.
pub trait Portal {
    /// `$LRFL_ACCOUNT`, or the default `lrfl config set-account` saved.
    fn default_account(&self) -> Option<String>;
    /// The accounts linked to the login; `None` when no login is configured.
    fn linked_accounts(&self) -> Result<Option<Vec<LinkedAccount>>, AppError>;
    fn utility_account(&self, id: &AccountId) -> Result<Value, AppError>;
    fn payment_history(&self, id: &AccountId, after: &str) -> Result<Value, AppError>;
}

/// `Portal` over the district's API.
pub struct LivePortal {
    api: Wipp,
    session: Session,
    /// `--email` or `$LRFL_EMAIL`, which win over the saved login name.
    login: Option<String>,
    linked: Mutex<Linked>,
}

/// What a login found, and the password FIS last refused, each with the login
/// it went with: a running server sees the sign-in the user does next.
#[derive(Default)]
struct Linked {
    /// Reading the list is a full sign-in, so it is kept for [`KEEP_LINKED`].
    kept: Option<(String, Instant, Vec<LinkedAccount>)>,
    /// Not sent again, so a server left running after a password change cannot
    /// lock the login by retrying it: the next call that finds a different
    /// password tries that. Only a refusal lands here; FIS being down does not.
    refused: Option<(String, Secret)>,
}

impl Linked {
    fn read(
        &mut self,
        login: &str,
        now: Instant,
        password: impl FnOnce() -> Result<Secret, AppError>,
        fetch: impl FnOnce(&Secret) -> Result<Vec<LinkedAccount>, AppError>,
    ) -> Result<Vec<LinkedAccount>, AppError> {
        if let Some((kept_for, at, accounts)) = &self.kept {
            if kept_for == login && now.duration_since(*at) < KEEP_LINKED {
                return Ok(accounts.clone());
            }
        }
        let password = password()?;
        let refused = self.refused.as_ref();
        if refused.is_some_and(|(l, r)| l == login && r.expose() == password.expose()) {
            return Err(AppError::Auth(
                "the district refused this password on an earlier call, so it was not sent again"
                    .into(),
            ));
        }
        self.kept = None;
        match fetch(&password) {
            Ok(accounts) => {
                self.kept = Some((login.to_string(), now, accounts.clone()));
                self.refused = None;
                Ok(accounts)
            }
            Err(e) => {
                if matches!(e, AppError::Auth(_)) {
                    self.refused = Some((login.to_string(), password));
                }
                Err(e)
            }
        }
    }
}

/// `fis_login` answers `Auth` for a refused password only. A token FIS just
/// issued being turned away by the next call says nothing about the password.
fn not_a_refusal(e: AppError) -> AppError {
    match e {
        AppError::Auth(message) => AppError::Upstream(message),
        other => other,
    }
}

impl LivePortal {
    fn password(&self, login: &str) -> Result<Secret, AppError> {
        // A variable set but empty is a usage error on a command line. Here no
        // tool argument can fix it, and the model would be told to try another.
        self.session.password(login).map_err(|e| match e {
            AppError::Usage(message) => AppError::Auth(message),
            other => other,
        })
    }
}

impl Portal for LivePortal {
    fn default_account(&self) -> Option<String> {
        std::env::var(ACCOUNT_ENV)
            .ok()
            .filter(|a| !a.trim().is_empty())
            .or_else(config::load_default_account)
    }

    fn linked_accounts(&self) -> Result<Option<Vec<LinkedAccount>>, AppError> {
        // No login name is a guest, which is not an error here.
        let Ok(login) = self.session.resolve_login(self.login.as_deref()) else {
            return Ok(None);
        };
        self.linked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .read(
                &login,
                Instant::now(),
                || self.password(&login),
                |password| {
                    let token = self.api.fis_login(&login, password.expose())?;
                    let body = self.api.billing_accounts(&token).map_err(not_a_refusal)?;
                    Ok(LinkedAccount::list_from(&body))
                },
            )
            .map(Some)
    }

    fn utility_account(&self, id: &AccountId) -> Result<Value, AppError> {
        self.api.utility_account(id)
    }

    fn payment_history(&self, id: &AccountId, after: &str) -> Result<Value, AppError> {
        self.api.payment_history(id, after)
    }
}

fn shape_error(what: &str) -> AppError {
    AppError::Upstream(format!(
        "the district's {what} did not have the expected shape, so it is not reported here"
    ))
}

/// [`UtilitySource`] over a [`Portal`].
pub struct LoxSource<P> {
    portal: P,
}

impl<P: Portal> LoxSource<P> {
    /// The named account, else the default, else the login's only one, the
    /// order the commands use.
    fn account(&self, named: Option<&str>) -> Result<AccountId, AppError> {
        if let Some(raw) = named {
            return AccountId::parse(raw);
        }
        if let Some(raw) = self.portal.default_account() {
            return AccountId::parse(&raw);
        }
        let linked: Vec<LinkedAccount> = self
            .portal
            .linked_accounts()?
            .unwrap_or_default()
            .into_iter()
            .filter(LinkedAccount::is_utility)
            .collect();
        match linked.as_slice() {
            [one] => AccountId::parse(&one.account_id),
            [] => Err(AppError::Usage(
                "no account: pass the account number as `account`, NNNNNNN-N. The district's \
                 lookup needs no sign-in"
                    .into(),
            )),
            many => Err(AppError::Usage(format!(
                "{} accounts are linked to this login: name one as `account`",
                many.len()
            ))),
        }
    }
}

impl<P: Portal> UtilitySource for LoxSource<P> {
    /// Reads are anonymous, so the list is only ever a convenience. With a default
    /// account nothing needs it, which also keeps that server off the keychain, and
    /// a failed sign-in must not cost the read of an account the call named. With no
    /// account to go on, [`LoxSource::account`] reports the failure itself.
    async fn accounts(&self) -> Result<Vec<UtilityAccount>, AppError> {
        if self.portal.default_account().is_some() {
            return Ok(Vec::new());
        }
        let linked = self.portal.linked_accounts().ok().flatten();
        Ok(linked
            .unwrap_or_default()
            .into_iter()
            .filter(LinkedAccount::is_utility)
            .map(|a| UtilityAccount {
                id: a.account_id,
                name: None,
            })
            .collect())
    }

    async fn summary(&self, account: Option<&str>) -> Result<UtilitySummary, AppError> {
        let id = self.account(account)?;
        let body = self.portal.utility_account(&id)?;
        if !account::balance_stated(&body) {
            return Err(shape_error("account record"));
        }
        Ok(formatter::utility_summary(&Account::from_node(
            &id.dashed(),
            &body,
        )))
    }

    async fn statements(&self, account: Option<&str>) -> Result<Vec<Statement>, AppError> {
        let id = self.account(account)?;
        let body = self.portal.utility_account(&id)?;
        if !bills::amounts_stated(&body) {
            return Err(shape_error("account record"));
        }
        Ok(formatter::statements_of(&BillPeriod::list_from_account(
            &body,
        )))
    }

    async fn payments(
        &self,
        account: Option<&str>,
        since: Option<&str>,
    ) -> Result<Vec<PaymentDto>, AppError> {
        let id = self.account(account)?;
        let after = since.map_or_else(|| util::years_ago(PAYMENT_YEARS), str::to_string);
        let body = self.portal.payment_history(&id, &after)?;
        let rows = body
            .as_array()
            .ok_or_else(|| shape_error("payment history"))?;
        // `Payment::from_node` reads a missing amount as 0.00 and a missing date
        // as blank, which a tool must not hand a model as fact.
        let readable = |r: &&Value| {
            r.get("amt").is_some_and(Value::is_number)
                && r.get("paymentDate")
                    .and_then(Value::as_str)
                    .is_some_and(|d| !d.trim().is_empty())
        };
        let mut payments: Vec<Payment> = rows
            .iter()
            .filter(readable)
            .map(Payment::from_node)
            .collect();
        if !rows.is_empty() && payments.is_empty() {
            return Err(shape_error("payment history"));
        }
        payments.sort_by(|a, b| {
            formatter::iso_date(&b.payment_date).cmp(&formatter::iso_date(&a.payment_date))
        });
        Ok(payments.iter().map(formatter::utility_payment).collect())
    }

    async fn usage(&self, account: Option<&str>) -> Result<Vec<UsagePeriod>, AppError> {
        let id = self.account(account)?;
        let body = self.portal.utility_account(&id)?;
        if !body.get("chargeTypes").is_some_and(Value::is_object) {
            return Err(shape_error("account record"));
        }
        Ok(BillPeriod::list_from_account(&body)
            .iter()
            .filter_map(formatter::usage_of)
            .collect())
    }
}

#[cfg(test)]
mod tests;
