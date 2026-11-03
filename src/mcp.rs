//! `lrfl mcp`: the shared `utility/v1` tools (`pk_cli_utility::mcp`) over the
//! district's WIPP API.
//!
//! This module supplies what is the district's own: [`Portal`], the reads the
//! commands already make, and [`LoxSource`], which maps them with the commands'
//! mappers in `formatter.rs`. Tests put fixtures behind [`Portal`], so nothing
//! here needs a network to test.
//!
//! Every read is the anonymous lookup by account number, so the server signs in
//! nowhere and holds no password. A login would add only the list of linked
//! accounts, which costs a password read and a full sign-in, and is not worth
//! those here: an account is named by its number, or set as the default.
//!
//! The commands' mappers are best-effort, for a person who can see the raw
//! payload. A tool's answer goes to a model as fact, so here an answer the
//! district did not give (a balance no field states, rows none of which can be
//! read) is an upstream error, never 0.00 or an empty list.

use pk_cli_mcp::Server;
use pk_cli_utility::mcp::{UtilityAccount, UtilitySource, UtilityTools};
use pk_cli_utility::{Payment as PaymentDto, Statement, UsagePeriod, UtilitySummary};
use serde_json::Value;

use crate::account::{self, Account};
use crate::acct::AccountId;
use crate::bills::{self, BillPeriod};
use crate::client::Wipp;
use crate::config;
use crate::error::AppError;
use crate::formatter;
use crate::model::Payment;
use crate::util;

/// The default account's environment variable, which a Claude Desktop
/// extension's settings fill in.
const ACCOUNT_ENV: &str = "LRFL_ACCOUNT";

/// How far back payments go when a call names no `since`, as `lrfl history`.
const PAYMENT_YEARS: u32 = 3;

/// The tools `lrfl mcp` serves. Nothing here reads a file: listing the tools
/// touches none, and a server left running sees the default the user sets next.
pub fn tools(wipp_id: String) -> Result<UtilityTools<LoxSource<LivePortal>>, AppError> {
    Ok(with_source(LoxSource {
        portal: LivePortal {
            api: Wipp::new(wipp_id)?,
        },
    }))
}

fn with_source<P: Portal>(source: LoxSource<P>) -> UtilityTools<LoxSource<P>> {
    let server = Server::new(
        "lrfl",
        env!("CARGO_PKG_VERSION"),
        "Loxahatchee River District",
        "loxahatchee",
    )
    // No credential is involved, so the standard "ask the user to sign in" would
    // send them to a command that changes nothing here.
    .hosted(
        "through the open-source `lrfl` command-line tool, by the district's anonymous account \
         lookup, with no credentials",
        "tell the user, since nothing here signs in and the district's lookup may have changed",
    )
    .instructions(
        "Loxahatchee River District access here is read-only: no tool can make a payment or \
         change anything. An account is read by its number, NNNNNNN-N, which a call names unless \
         a default is set. Bills are the current period and up to three before it, which is all \
         the district reports here. Usage exists only for metered services, in the district's \
         own meter units, so an unmetered account has none.",
    );
    UtilityTools::new(server, source).metering("metered service use")
}

/// The district reads the tools need. The JSON is the district's own, as the
/// models parse it.
pub trait Portal {
    /// `$LRFL_ACCOUNT`, or the default `lrfl config set-account` saved.
    fn default_account(&self) -> Option<String>;
    fn utility_account(&self, id: &AccountId) -> Result<Value, AppError>;
    fn payment_history(&self, id: &AccountId, after: &str) -> Result<Value, AppError>;
}

/// `Portal` over the district's API.
pub struct LivePortal {
    api: Wipp,
}

impl Portal for LivePortal {
    fn default_account(&self) -> Option<String> {
        std::env::var(ACCOUNT_ENV)
            .ok()
            .filter(|a| !a.trim().is_empty())
            .or_else(config::load_default_account)
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
    /// The named account, else the default.
    fn account(&self, named: Option<&str>) -> Result<AccountId, AppError> {
        let default = self.portal.default_account();
        match named.or(default.as_deref()) {
            Some(raw) => AccountId::parse(raw),
            None => Err(AppError::Usage(
                "no account: pass the account number as `account`, NNNNNNN-N".into(),
            )),
        }
    }
}

impl<P: Portal> UtilitySource for LoxSource<P> {
    /// No list, so a call may name any account number, as `lrfl balance` may: the
    /// district answers that lookup without a sign-in.
    async fn accounts(&self) -> Result<Vec<UtilityAccount>, AppError> {
        Ok(Vec::new())
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
