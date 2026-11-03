//! `bill` — the account's current bill, parsed from the official PDF.

use crate::account::Account;
use crate::acct::AccountId;
use crate::bill::Bill;
use crate::cli::AccountArg;
use crate::commands::Ctx;
use crate::error::AppError;
use crate::formatter;
use crate::util;

/// Resolve the URL of an account's current bill PDF (it is keyed by the account's
/// current due date, which we read from the account record first).
pub fn current_url(ctx: &Ctx, id: &AccountId) -> Result<String, AppError> {
    let acct = Account::from_node(&id.dashed(), &ctx.api.utility_account(id)?);
    let due = acct
        .charges
        .iter()
        .map(|c| c.current_due_date.trim())
        .find(|d| !d.is_empty())
        .ok_or_else(|| {
            AppError::NotFound(format!("no current bill for {} (nothing due)", id.dashed()))
        })?;
    ctx.api.bill_url(id, due)
}

/// Fetch and parse an account's current bill PDF. Shared by `bill` and by
/// `search --full` (which enriches each match with the same PDF-only fields).
pub fn fetch(ctx: &Ctx, id: &AccountId) -> Result<Bill, AppError> {
    let url = current_url(ctx, id)?;
    let bytes = ctx.api.fetch_bytes(&url)?;
    let text = pdf_extract::extract_text_from_mem(&bytes)
        .map_err(|e| AppError::Other(format!("reading the bill PDF: {e}")))?;
    Ok(Bill::parse(&text))
}

pub fn run(ctx: &Ctx, arg: &AccountArg, open: bool, save: Option<&str>) -> Result<(), AppError> {
    let id = ctx.resolve_account(arg)?;
    ctx.log(&format!("resolving bill for {}", id.dashed()));

    let url = current_url(ctx, &id)?;

    if open {
        crate::client::require_secure_document_url(&url)?;
        util::open_url(&url)?;
        if ctx.json {
            println!(
                "{}",
                serde_json::json!({ "account": id.dashed(), "opened": url })
            );
        } else if !ctx.quiet {
            println!("Opened the bill PDF for {}", id.dashed());
        }
        return Ok(());
    }

    let bytes = ctx.api.fetch_bytes(&url)?;

    if let Some(path) = save {
        std::fs::write(path, &bytes)
            .map_err(|e| AppError::Other(format!("writing {path}: {e}")))?;
        if ctx.json {
            println!(
                "{}",
                serde_json::json!({ "account": id.dashed(), "saved": path, "bytes": bytes.len() })
            );
        } else if !ctx.quiet {
            println!("Saved the bill PDF to {path} ({} bytes)", bytes.len());
        }
        return Ok(());
    }

    let text = pdf_extract::extract_text_from_mem(&bytes)
        .map_err(|e| AppError::Other(format!("reading the bill PDF: {e}")))?;
    let bill = Bill::parse(&text);
    formatter::print_bill(&bill, ctx.json);
    Ok(())
}
