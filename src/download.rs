//! Downloading a period's hosted bill PDF — the file mechanism shared by the
//! `bills` (utility-flavored) and `documents` (documents/v1) command surfaces.
//!
//! The command modules own only argument-wrangling and DTO rendering; the
//! fetch → write → skip-collect loop lives here so the two spellings of the
//! same download can't drift apart (they once did, on skip handling). Nothing
//! here depends on [`crate::commands`] — it takes the API client
//! ([`Wipp`](crate::client::Wipp)) directly, so there is no command→command or
//! domain→command edge.

use std::path::{Path, PathBuf};

use crate::acct::AccountId;
use crate::bills::BillPeriod;
use crate::client::Wipp;
use crate::error::AppError;

/// One successfully written PDF. The command layer maps this onto its own DTO
/// (`bill-download/v1` fields for `bills`, `document-download/v1` for
/// `documents`).
pub struct Saved {
    /// Period id — the ISO due date.
    pub due: String,
    /// The hosted-PDF URL the bytes came from.
    pub source_url: String,
    /// Where the file was written on disk.
    pub path: String,
    pub bytes: u64,
}

/// A listed period a `--all` run couldn't produce a file for.
pub struct Skipped {
    /// Period id — the ISO due date.
    pub due: String,
    /// Stable category slug, for machine branching (`no_file` today — the
    /// archive has no PDF on record for this period).
    pub code: &'static str,
    /// Human-readable explanation.
    pub reason: String,
}

/// Deduped, newest-first due dates across an account's periods, optionally
/// filtered to one service by name. WIPP serves one hosted PDF per due date
/// even when services share the period (e.g. Water + Sewer), so the list of
/// downloadable files is keyed by date, not by period row.
pub fn due_dates(periods: &[BillPeriod], service: Option<&str>) -> Vec<String> {
    let mut dues: Vec<String> = periods
        .iter()
        .filter(|p| service.map_or(true, |s| p.service.eq_ignore_ascii_case(s.trim())))
        .map(|p| p.due_date.clone())
        .collect();
    dues.sort();
    dues.dedup();
    dues.reverse(); // newest first (matches `bills list` / `documents list`)
    dues
}

/// Resolve one period's hosted URL and download its PDF, returning
/// `(source_url, bytes)`. Returns [`AppError::NotFound`] when the archive
/// serves a placeholder rather than a real PDF (see
/// [`crate::client::Wipp::fetch_bill_pdf`]).
pub fn fetch_period(api: &Wipp, id: &AccountId, due: &str) -> Result<(String, Vec<u8>), AppError> {
    let url = api.bill_url(id, due)?;
    let bytes = api.fetch_bill_pdf(&url, due)?;
    Ok((url, bytes))
}

/// Default per-file name: `lrfl-bill-<account>-<due>.pdf`.
pub fn default_filename(id: &AccountId, due: &str) -> String {
    format!("lrfl-bill-{}-{}.pdf", id.dashed(), due)
}

/// A file path as given; if it is an existing directory, join the default
/// filename into it; otherwise treat the argument as the exact file target.
pub fn resolve_target(output: Option<&str>, default: &str) -> PathBuf {
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

/// Create `dir` if it is a non-empty path that doesn't exist yet. A no-op for
/// the empty path (the current directory).
pub fn ensure_dir(dir: &Path) -> Result<(), AppError> {
    if !dir.as_os_str().is_empty() && !dir.exists() {
        std::fs::create_dir_all(dir)
            .map_err(|e| AppError::Other(format!("creating {}: {e}", dir.display())))?;
    }
    Ok(())
}

/// The batch DTO's `dir` label: `.` for the empty path (the current dir),
/// otherwise the path as written. Pairs with [`ensure_dir`] so the "empty means
/// current directory" convention lives in one place, not re-spelled per caller.
pub fn dir_label(dir: &Path) -> String {
    if dir.as_os_str().is_empty() {
        ".".to_string()
    } else {
        dir.display().to_string()
    }
}

/// Fetch and write every `due` into `dir` (empty = current dir), collecting
/// missing periods as [`Skipped`] rather than aborting so one gap doesn't kill
/// the batch. `progress` is invoked with each due date before it is fetched, so
/// the caller can wire `--verbose`. Any error other than [`AppError::NotFound`]
/// aborts the batch (a real fault, not a missing file).
pub fn download_periods(
    api: &Wipp,
    id: &AccountId,
    dues: &[String],
    dir: &Path,
    progress: impl Fn(&str),
) -> Result<(Vec<Saved>, Vec<Skipped>), AppError> {
    let mut saved = Vec::new();
    let mut skipped = Vec::new();
    for due in dues {
        progress(due);
        match fetch_period(api, id, due) {
            Ok((source_url, bytes)) => {
                let file = default_filename(id, due);
                let target = if dir.as_os_str().is_empty() {
                    PathBuf::from(&file)
                } else {
                    dir.join(&file)
                };
                std::fs::write(&target, &bytes)
                    .map_err(|e| AppError::Other(format!("writing {}: {e}", target.display())))?;
                saved.push(Saved {
                    due: due.clone(),
                    source_url,
                    path: target.display().to_string(),
                    bytes: bytes.len() as u64,
                });
            }
            Err(AppError::NotFound(msg)) => skipped.push(Skipped {
                due: due.clone(),
                code: "no_file",
                reason: msg,
            }),
            Err(e) => return Err(e),
        }
    }
    Ok((saved, skipped))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn period(due: &str, service: &str) -> BillPeriod {
        BillPeriod {
            due_date: due.to_string(),
            service: service.to_string(),
            amount: 0.0,
            interest: 0.0,
            principal_balance: 0.0,
            paid: false,
            current: false,
            period_start: String::new(),
            period_end: String::new(),
            reading_date: String::new(),
            reading: None,
            usage: None,
        }
    }

    #[test]
    fn due_dates_dedupe_by_date_and_return_newest_first() {
        // Two services share each due date; input order is arbitrary.
        let periods = vec![
            period("2026-02-11", "Sewer"),
            period("2026-08-12", "Water"),
            period("2026-05-13", "Sewer"),
            period("2026-08-12", "Sewer"),
            period("2026-05-13", "Water"),
        ];
        assert_eq!(
            due_dates(&periods, None),
            vec!["2026-08-12", "2026-05-13", "2026-02-11"]
        );
    }

    #[test]
    fn due_dates_filter_by_service_case_insensitively() {
        let periods = vec![
            period("2026-08-12", "Water"),
            period("2026-08-12", "Sewer"),
            period("2026-05-13", "Sewer"),
        ];
        // Only Sewer's dates, still deduped + newest-first.
        assert_eq!(
            due_dates(&periods, Some("  sewer ")),
            vec!["2026-08-12", "2026-05-13"]
        );
    }

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
