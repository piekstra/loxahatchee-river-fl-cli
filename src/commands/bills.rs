//! `bills` — historical bill periods for an account.
//!
//! - `bills list` enumerates the periods the WIPP API carries on the account
//!   payload (current + up to three prior per service, keyed by ISO due date).
//! - `bills get <YYYY-MM-DD>` (or `--all`) downloads the district's official
//!   PDF for that period via the same anonymous hosted-PDF chain `bill` uses.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::acct::AccountId;
use crate::bills::BillPeriod;
use crate::cli::{AccountArg, BillsCmd};
use crate::commands::Ctx;
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
    let (path, bytes) = fetch_period(ctx, &id, &due)?;

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

    let default = default_filename(&id, &due);
    let target = resolve_target(output, &default);
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

    // Dedupe by due date: WIPP serves one hosted PDF per due date, even when
    // multiple services share the period (e.g. Water + Sewer). Fetching the
    // same PDF once and per service would just overwrite the same file.
    let mut due_dates: Vec<String> = periods.iter().map(|p| p.due_date.clone()).collect();
    due_dates.sort();
    due_dates.dedup();

    let dir = output.map(PathBuf::from).unwrap_or_default();
    if !dir.as_os_str().is_empty() && !dir.exists() {
        std::fs::create_dir_all(&dir)
            .map_err(|e| AppError::Other(format!("creating {}: {e}", dir.display())))?;
    }

    let mut items = Vec::new();
    let mut bytes_total: u64 = 0;
    let mut skipped: Vec<(String, String)> = Vec::new();
    for due in &due_dates {
        ctx.log(&format!("fetching bill {due}"));
        match fetch_period(ctx, &id, due) {
            Ok((source_url, bytes)) => {
                let file = default_filename(&id, due);
                let target = if dir.as_os_str().is_empty() {
                    PathBuf::from(&file)
                } else {
                    dir.join(&file)
                };
                std::fs::write(&target, &bytes)
                    .map_err(|e| AppError::Other(format!("writing {}: {e}", target.display())))?;
                bytes_total += bytes.len() as u64;
                items.push(json!({
                    "period_id": due,
                    "source_url": source_url,
                    "path": target.display().to_string(),
                    "bytes": bytes.len(),
                }));
            }
            Err(AppError::NotFound(msg)) => {
                // A period is listed but no PDF is on file (rare — LOXA's
                // archive usually keeps at least the 3-prior window). Record
                // and keep going so one gap doesn't abort the batch.
                skipped.push((due.clone(), msg));
            }
            Err(e) => return Err(e),
        }
    }

    let where_to = if dir.as_os_str().is_empty() {
        ".".to_string()
    } else {
        dir.display().to_string()
    };
    let dto = json!({
        "schema": "bill-download-batch/v1",
        "account": id.dashed(),
        "count": items.len(),
        "bytes_total": bytes_total,
        "dir": where_to,
        "items": items,
        "skipped": skipped
            .iter()
            .map(|(d, m)| json!({ "period_id": d, "reason": m }))
            .collect::<Vec<_>>(),
    });
    if ctx.json {
        println!("{}", serde_json::to_string_pretty(&dto).expect("json"));
    } else if !ctx.quiet {
        for item in &items {
            println!(
                "Saved {} → {} ({} bytes)",
                item.get("period_id").and_then(|v| v.as_str()).unwrap_or(""),
                item.get("path").and_then(|v| v.as_str()).unwrap_or(""),
                item.get("bytes").and_then(|v| v.as_u64()).unwrap_or(0),
            );
        }
        for (due, msg) in &skipped {
            eprintln!("skipped {due}: {msg}");
        }
        println!("{} bill(s), {bytes_total} bytes → {where_to}", items.len());
    }
    Ok(())
}

/// Resolve one period's hosted URL, download the PDF, and return
/// `(source_url, bytes)`. Returns [`AppError::NotFound`] if the archive
/// serves a placeholder (see [`crate::client::Wipp::fetch_bill_pdf`]).
pub(crate) fn fetch_period(
    ctx: &Ctx,
    id: &AccountId,
    due: &str,
) -> Result<(String, Vec<u8>), AppError> {
    let url = ctx.api.bill_url(id, due)?;
    let bytes = ctx.api.fetch_bill_pdf(&url, due)?;
    Ok((url, bytes))
}

pub(crate) fn default_filename(id: &AccountId, due: &str) -> String {
    format!("lrfl-bill-{}-{}.pdf", id.dashed(), due)
}

/// A file path as given; if the path is an existing directory, join the default
/// filename into it; otherwise treat the argument as the exact file target.
pub(crate) fn resolve_target(output: Option<&str>, default: &str) -> PathBuf {
    match output {
        None => PathBuf::from(default),
        Some(s) => {
            let p = Path::new(s);
            if p.is_dir() {
                p.join(default)
            } else {
                p.to_path_buf()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{default_filename, resolve_target};
    use crate::acct::AccountId;
    use std::path::PathBuf;

    #[test]
    fn default_filename_includes_account_and_period() {
        let id = AccountId::parse("1234567-8").unwrap();
        assert_eq!(
            default_filename(&id, "2026-05-13"),
            "lrfl-bill-1234567-8-2026-05-13.pdf"
        );
    }

    #[test]
    fn resolve_target_uses_default_when_output_absent() {
        assert_eq!(
            resolve_target(None, "lrfl-bill-1234567-8-2026-05-13.pdf"),
            PathBuf::from("lrfl-bill-1234567-8-2026-05-13.pdf")
        );
    }

    #[test]
    fn resolve_target_takes_explicit_file_verbatim() {
        assert_eq!(
            resolve_target(Some("/tmp/x.pdf"), "lrfl-bill-1234567-8-2026-05-13.pdf"),
            PathBuf::from("/tmp/x.pdf")
        );
    }

    #[test]
    fn resolve_target_joins_filename_onto_directory() {
        // `.` is a stable directory that always exists.
        assert_eq!(
            resolve_target(Some("."), "lrfl-bill-1234567-8-2026-05-13.pdf"),
            PathBuf::from("./lrfl-bill-1234567-8-2026-05-13.pdf")
        );
    }
}
