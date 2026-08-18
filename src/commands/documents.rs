//! `documents` — the district's official bill PDFs as the documents/v1 profile.
//!
//! Loxahatchee statements are the hosted bill PDFs, keyed by ISO due date, so a
//! document's id is that due date. `documents list` enumerates the downloadable
//! periods; `documents download <id>` fetches the PDF — the same file
//! `bills get <id>` produces, over the shared [`crate::download`] core (so the
//! two surfaces can't drift). Only the file surface is here; `bills list`
//! remains the utility/v1 statement view.

use std::io::Write;
use std::path::PathBuf;

use pk_cli_documents::{Document, DownloadBatch, Paged, SavedDocument, SkippedDocument};

use crate::bills::BillPeriod;
use crate::cli::{AccountArg, DocumentsCmd};
use crate::commands::Ctx;
use crate::download;
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
    let id = ctx.resolve_account(account)?;
    let periods = BillPeriod::list_from_account(&ctx.api.utility_account(&id)?);
    let dues = download::due_dates(&periods, service);
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
    let (_source_url, bytes) = download::fetch_period(&ctx.api, &acct_id, &due)?;

    if output == Some("-") {
        return std::io::stdout()
            .write_all(&bytes)
            .map_err(|e| AppError::Other(format!("writing PDF to stdout: {e}")));
    }
    let default = download::default_filename(&acct_id, &due);
    let target = download::resolve_target(output, &default);
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
    let acct_id = ctx.resolve_account(account)?;
    let periods = BillPeriod::list_from_account(&ctx.api.utility_account(&acct_id)?);
    let dues = download::due_dates(&periods, None);

    let dir = output.map(PathBuf::from).unwrap_or_default();
    download::ensure_dir(&dir)?;
    let (saved, skipped) = download::download_periods(&ctx.api, &acct_id, &dues, &dir, |d| {
        ctx.log(&format!("fetching bill {d}"))
    })?;

    let where_to = if dir.as_os_str().is_empty() {
        ".".to_string()
    } else {
        dir.display().to_string()
    };
    let items: Vec<SavedDocument> = saved
        .iter()
        .map(|s| saved_doc(&s.due, &s.path, s.bytes as usize))
        .collect();
    // Surface listed periods with no PDF on file instead of silently coming up
    // short — the same partial-failure reporting `bills download --all` gives.
    let skips: Vec<SkippedDocument> = skipped
        .iter()
        .map(|s| SkippedDocument::new(s.due.clone(), s.reason.clone()).with_code(s.code))
        .collect();
    let batch = DownloadBatch::new(where_to, items).with_skipped(skips);
    if ctx.json {
        println!("{}", serde_json::to_string_pretty(&batch).expect("json"));
    } else if !ctx.quiet {
        for it in &batch.items {
            println!("Saved {} → {} ({} bytes)", it.id, it.path, it.bytes);
        }
        for s in &batch.skipped {
            eprintln!("skipped {}: {}", s.id, s.reason);
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
