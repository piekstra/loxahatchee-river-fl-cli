//! `bills` — historical bill periods for an account.
//!
//! - `bills list` enumerates the periods the WIPP API carries on the account
//!   payload (current + up to three prior per service, keyed by ISO due date).
//! - `bills get <YYYY-MM-DD>` (or `--all`) downloads the district's official
//!   PDF for that period via the same anonymous hosted-PDF chain `bill` uses.

use std::io::Write;
use std::path::PathBuf;

use serde_json::json;

use crate::bills::BillPeriod;
use crate::cli::{AccountArg, BillsCmd};
use crate::commands::Ctx;
use crate::download;
use crate::error::AppError;
use crate::formatter;

pub fn run(ctx: &Ctx, cmd: &BillsCmd) -> Result<(), AppError> {
    match cmd {
        BillsCmd::List { account, service } => list(ctx, account, service.as_deref()),
        BillsCmd::Get {
            period_id,
            account,
            all,
            output,
        } => {
            // The subcommand keeps its account as a plain positional (so
            // `period_id` can come first); wrap it into an `AccountArg` for
            // the shared resolver.
            let acct = AccountArg {
                account: account.clone(),
            };
            get(ctx, &acct, period_id.as_deref(), *all, output.as_deref())
        }
    }
}

fn list(ctx: &Ctx, account: &AccountArg, service: Option<&str>) -> Result<(), AppError> {
    let id = ctx.resolve_account(account)?;
    ctx.log(&format!("enumerating bill periods for {}", id.dashed()));
    let acct = ctx.api.utility_account(&id)?;
    let mut periods = BillPeriod::list_from_account(&acct);
    if let Some(svc) = service {
        let want = svc.trim().to_ascii_lowercase();
        periods.retain(|p| p.service.eq_ignore_ascii_case(&want));
    }
    formatter::print_bills(&id.dashed(), &periods, ctx.json);
    Ok(())
}

fn get(
    ctx: &Ctx,
    account: &AccountArg,
    period_id: Option<&str>,
    all: bool,
    output: Option<&str>,
) -> Result<(), AppError> {
    match (all, period_id) {
        (true, _) => download_all(ctx, account, output),
        (false, Some(id)) => download_one(ctx, account, id, output),
        (false, None) => Err(AppError::Usage(
            "give a period id (see `lrfl bills list`) or --all".into(),
        )),
    }
}

fn download_one(
    ctx: &Ctx,
    account: &AccountArg,
    period_id: &str,
    output: Option<&str>,
) -> Result<(), AppError> {
    let id = ctx.resolve_account(account)?;
    let due = crate::util::validate_date(period_id).map_err(|_| {
        AppError::Usage(format!(
            "period id must be an ISO due date YYYY-MM-DD (see `lrfl bills list`), got {period_id:?}"
        ))
    })?;
    let (path, bytes) = download::fetch_period(&ctx.api, &id, &due)?;

    if output == Some("-") {
        std::io::stdout()
            .write_all(&bytes)
            .map_err(|e| AppError::Other(format!("writing PDF to stdout: {e}")))?;
        if !ctx.quiet {
            eprintln!(
                "wrote {} bytes ({} bill for {}) to stdout",
                bytes.len(),
                due,
                id.dashed()
            );
        }
        return Ok(());
    }

    let default = download::default_filename(&id, &due);
    let target = download::resolve_target(output, &default);
    std::fs::write(&target, &bytes)
        .map_err(|e| AppError::Other(format!("writing {}: {e}", target.display())))?;

    let dto = json!({
        "schema": "bill-download/v1",
        "account": id.dashed(),
        "period_id": due,
        "source_url": path,
        "path": target.display().to_string(),
        "bytes": bytes.len(),
    });
    if ctx.json {
        println!("{}", serde_json::to_string_pretty(&dto).expect("json"));
    } else if !ctx.quiet {
        println!(
            "Saved bill for {} ({}) → {} ({} bytes)",
            id.dashed(),
            due,
            target.display(),
            bytes.len()
        );
    }
    Ok(())
}

fn download_all(ctx: &Ctx, account: &AccountArg, output: Option<&str>) -> Result<(), AppError> {
    if output == Some("-") {
        return Err(AppError::Usage(
            "--all can't stream to stdout; give a directory with -o, or omit it".into(),
        ));
    }
    let id = ctx.resolve_account(account)?;
    let periods = BillPeriod::list_from_account(&ctx.api.utility_account(&id)?);
    let dues = download::due_dates(&periods, None);

    let dir = output.map(PathBuf::from).unwrap_or_default();
    download::ensure_dir(&dir)?;
    let (saved, skipped) = download::download_periods(&ctx.api, &id, &dues, &dir, |d| {
        ctx.log(&format!("fetching bill {d}"))
    })?;

    let where_to = if dir.as_os_str().is_empty() {
        ".".to_string()
    } else {
        dir.display().to_string()
    };
    let bytes_total: u64 = saved.iter().map(|s| s.bytes).sum();
    let dto = json!({
        "schema": "bill-download-batch/v1",
        "account": id.dashed(),
        "count": saved.len(),
        "bytes_total": bytes_total,
        "dir": where_to,
        "items": saved
            .iter()
            .map(|s| json!({
                "period_id": s.due,
                "source_url": s.source_url,
                "path": s.path,
                "bytes": s.bytes,
            }))
            .collect::<Vec<_>>(),
        "skipped": skipped
            .iter()
            .map(|s| json!({ "period_id": s.due, "reason": s.reason }))
            .collect::<Vec<_>>(),
    });
    if ctx.json {
        println!("{}", serde_json::to_string_pretty(&dto).expect("json"));
    } else if !ctx.quiet {
        for s in &saved {
            println!("Saved {} → {} ({} bytes)", s.due, s.path, s.bytes);
        }
        for s in &skipped {
            eprintln!("skipped {}: {}", s.due, s.reason);
        }
        println!("{} bill(s), {bytes_total} bytes → {where_to}", saved.len());
    }
    Ok(())
}
