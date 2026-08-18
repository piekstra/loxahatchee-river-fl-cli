//! `documents` — the district's official bill PDFs as the documents/v1 profile.
//!
//! Loxahatchee statements are the hosted bill PDFs, keyed by ISO due date, so a
//! document's id is that due date. `documents list` enumerates the downloadable
//! periods; `documents download <id>` fetches the PDF — the same file
//! `bills get <id>` produces (reuses `bills::fetch_period`). Only the file
//! surface is here; `bills list` remains the utility/v1 statement view.

use std::io::Write;
use std::path::PathBuf;

use pk_cli_documents::{Document, DownloadBatch, Paged, SavedDocument};

use crate::acct::AccountId;
use crate::bills::BillPeriod;
use crate::cli::{AccountArg, DocumentsCmd};
use crate::commands::bills::{default_filename, fetch_period, resolve_target};
use crate::commands::Ctx;
use crate::error::AppError;

pub fn run(ctx: &Ctx, cmd: &DocumentsCmd) -> Result<(), AppError> {
    match cmd {
        DocumentsCmd::List { account, service } => list(ctx, account, service.as_deref()),
        DocumentsCmd::Download {
            id,
            account,
            all,
            output,
        } => {
            let acct = AccountArg {
                account: account.clone(),
            };
            download(ctx, &acct, id.as_deref(), *all, output.as_deref())
        }
    }
}

/// Downloadable periods for an account, deduped by ISO due date (one hosted PDF
/// per date) and newest-first.
fn document_periods(
    ctx: &Ctx,
    account: &AccountArg,
    service: Option<&str>,
) -> Result<(AccountId, Vec<String>), AppError> {
    let id = ctx.resolve_account(account)?;
    let mut periods = BillPeriod::list_from_account(&ctx.api.utility_account(&id)?);
    if let Some(svc) = service {
        let want = svc.trim().to_ascii_lowercase();
        periods.retain(|p| p.service.eq_ignore_ascii_case(&want));
    }
    let mut dues: Vec<String> = periods.iter().map(|p| p.due_date.clone()).collect();
    dues.sort();
    dues.dedup();
    dues.reverse(); // newest first
    Ok((id, dues))
}

/// A due date → a documents/v1 [`Document`] (id = ISO due date; no financial
/// fields — an amount belongs to the utility/v1 statement, not the file).
fn document_of(due: &str) -> Document {
    let mut d = Document::new(
        due.to_string(),
        format!("Loxahatchee River District bill {due}"),
    );
    d.date = Some(due.to_string());
    d.category = Some("bill".into());
    d
}

fn saved_doc(due: &str, path: &str, bytes: usize) -> SavedDocument {
    SavedDocument::from_document(&document_of(due), path.to_string(), bytes as u64)
}

fn list(ctx: &Ctx, account: &AccountArg, service: Option<&str>) -> Result<(), AppError> {
    let (_id, dues) = document_periods(ctx, account, service)?;
    let docs: Vec<Document> = dues.iter().map(|d| document_of(d)).collect();
    if ctx.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&Paged::new("document", docs)).expect("json")
        );
    } else if docs.is_empty() {
        if !ctx.quiet {
            eprintln!("no downloadable statements found");
        }
    } else {
        for d in &docs {
            println!("{}\t{}", d.date.clone().unwrap_or_default(), d.name);
        }
    }
    Ok(())
}

fn download(
    ctx: &Ctx,
    account: &AccountArg,
    id: Option<&str>,
    all: bool,
    output: Option<&str>,
) -> Result<(), AppError> {
    if all {
        return download_all(ctx, account, output);
    }
    let acct_id = ctx.resolve_account(account)?;
    let due = match id {
        Some(id) => crate::util::validate_date(id).map_err(|_| {
            AppError::Usage(format!(
                "id must be an ISO due date YYYY-MM-DD (see `lrfl documents list`), got {id:?}"
            ))
        })?,
        None => {
            return Err(AppError::Usage(
                "give a document id (see `lrfl documents list`) or --all".into(),
            ))
        }
    };
    let (_source_url, bytes) = fetch_period(ctx, &acct_id, &due)?;

    if output == Some("-") {
        return std::io::stdout()
            .write_all(&bytes)
            .map_err(|e| AppError::Other(format!("writing PDF to stdout: {e}")));
    }
    let default = default_filename(&acct_id, &due);
    let target = resolve_target(output, &default);
    std::fs::write(&target, &bytes)
        .map_err(|e| AppError::Other(format!("writing {}: {e}", target.display())))?;

    let saved = saved_doc(&due, &target.display().to_string(), bytes.len());
    if ctx.json {
        println!("{}", serde_json::to_string_pretty(&saved).expect("json"));
    } else if !ctx.quiet {
        println!(
            "Saved bill {} → {} ({} bytes)",
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
    let (acct_id, dues) = document_periods(ctx, account, None)?;
    let dir = output.map(PathBuf::from).unwrap_or_default();
    if !dir.as_os_str().is_empty() && !dir.exists() {
        std::fs::create_dir_all(&dir)
            .map_err(|e| AppError::Other(format!("creating {}: {e}", dir.display())))?;
    }

    let mut items = Vec::new();
    for due in &dues {
        ctx.log(&format!("fetching bill {due}"));
        match fetch_period(ctx, &acct_id, due) {
            Ok((_url, bytes)) => {
                let file = default_filename(&acct_id, due);
                let target = if dir.as_os_str().is_empty() {
                    PathBuf::from(&file)
                } else {
                    dir.join(&file)
                };
                std::fs::write(&target, &bytes)
                    .map_err(|e| AppError::Other(format!("writing {}: {e}", target.display())))?;
                items.push(saved_doc(due, &target.display().to_string(), bytes.len()));
            }
            // A listed period with no PDF on file: skip so one gap doesn't
            // abort the batch (mirrors `bills download --all`).
            Err(AppError::NotFound(_)) => continue,
            Err(e) => return Err(e),
        }
    }

    let where_to = if dir.as_os_str().is_empty() {
        ".".to_string()
    } else {
        dir.display().to_string()
    };
    let batch = DownloadBatch::new(where_to, items);
    if ctx.json {
        println!("{}", serde_json::to_string_pretty(&batch).expect("json"));
    } else if !ctx.quiet {
        for it in &batch.items {
            println!("Saved {} → {} ({} bytes)", it.id, it.path, it.bytes);
        }
        println!(
            "{} bill(s), {} bytes → {}",
            batch.count, batch.bytes_total, batch.dir
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::document_of;

    #[test]
    fn document_of_conforms_to_documents_v1() {
        let v = serde_json::to_value(document_of("2026-05-13")).unwrap();
        assert_eq!(v["id"], "2026-05-13");
        assert_eq!(v["date"], "2026-05-13");
        assert_eq!(v["category"], "bill");
        assert!(
            v.get("amount").is_none(),
            "no financial fields on a document"
        );
        let doc: pk_cli_documents::Document = serde_json::from_value(v).unwrap();
        assert_eq!(doc.name, "Loxahatchee River District bill 2026-05-13");
    }
}
