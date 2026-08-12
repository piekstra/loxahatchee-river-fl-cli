//! Historical bill periods extracted from a utility account.
//!
//! The WIPP API doesn't expose an "all bills for this account" endpoint. What it
//! does carry, on the account payload, is the **current** billing period plus
//! three trailing **prior** periods per service (`priorDueDate1..3`,
//! `priorPrdBilled1..3`, `priorPrdInt1..3`, `priorPrdPrnBal1..3`, plus prior
//! meter readings/usage where metered). For LOXA (quarterly sewer), that adds
//! up to four periods per service — enough to satisfy the family's
//! `bills list` / `bills get <id>` surface without inventing history the
//! provider does not carry.
//!
//! Each period is keyed by its **due date** (ISO `YYYY-MM-DD`), which is what
//! `retrieveThirdPartyBillUrl?dueDate=…` (see [`crate::client::Wipp::bill_url`])
//! uses to fetch the hosted PDF for that period. The due date is the natural id.

use serde::Serialize;
use serde_json::Value;

/// One billing period for one service — a row in `lrfl bills list` and the unit
/// `lrfl bills get <id>` downloads.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct BillPeriod {
    /// Due date, ISO `YYYY-MM-DD`. Used as the id — this is what the WIPP
    /// hosted-PDF endpoint (`retrieveThirdPartyBillUrl?dueDate=…`) keys off.
    pub due_date: String,
    /// Service name, trimmed (`Sewer`, `Water`, `Electric`, `Other`).
    pub service: String,
    /// Amount billed for this period on this service.
    pub amount: f64,
    /// Interest billed for the period, when non-zero.
    #[serde(skip_serializing_if = "is_zero")]
    pub interest: f64,
    /// Principal balance still owed on this period (0 once paid off).
    #[serde(skip_serializing_if = "is_zero")]
    pub principal_balance: f64,
    /// Whether this period has been paid off (`principal_balance` is zero and
    /// it isn't the current outstanding period). Best-effort — the API doesn't
    /// carry an explicit "paid" flag per prior period.
    pub paid: bool,
    /// `true` for the account's current outstanding period; `false` for priors.
    pub current: bool,
    /// Service period start (ISO), where the API carries it (current period only).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub period_start: String,
    /// Service period end (ISO), where the API carries it (current period only).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub period_end: String,
    /// Meter reading date (metered services only), ISO.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub reading_date: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reading: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<i64>,
}

impl BillPeriod {
    /// Build every discoverable period from a raw `GET /wippUtil/{id}` body —
    /// one current + up to three prior periods per service, sorted newest-first.
    /// Duplicate due dates (rare — same due date on two services in the same
    /// period) are kept as separate rows so nothing gets silently coalesced.
    pub fn list_from_account(v: &Value) -> Vec<BillPeriod> {
        let mut out: Vec<BillPeriod> = v
            .get("chargeTypes")
            .and_then(Value::as_object)
            .map(|m| {
                m.iter()
                    .flat_map(|(name, node)| service_periods(name.trim(), node))
                    .collect()
            })
            .unwrap_or_default();
        // Newest first, then by service for stable ordering when dates tie.
        out.sort_by(|a, b| b.due_date.cmp(&a.due_date).then(a.service.cmp(&b.service)));
        out
    }

    /// Look one period up by its due-date id (`YYYY-MM-DD`).
    pub fn find<'a>(periods: &'a [BillPeriod], due_date: &str) -> Option<&'a BillPeriod> {
        periods.iter().find(|p| p.due_date == due_date)
    }
}

/// Extract the current period + up to three prior periods for one service.
fn service_periods(service: &str, node: &Value) -> Vec<BillPeriod> {
    let mut out = Vec::with_capacity(4);

    // Current period. The API also carries a `currPrdBilled` for a plain billed
    // amount — we prefer it, but fall back to `currPrdPrnBal` (still outstanding
    // principal, which for an unpaid current period equals the bill).
    if let Some(due) = clean_date(string_at(node, "currDueDate")) {
        let billed = float_at(node, "currPrdBilled");
        let principal_balance = float_at(node, "currPrdPrnBal");
        let interest = float_at(node, "currPrdInt");
        let amount = if billed > 0.0 {
            billed
        } else {
            principal_balance
        };
        out.push(BillPeriod {
            due_date: due,
            service: service.to_string(),
            amount,
            interest,
            principal_balance,
            paid: false,
            current: true,
            period_start: clean_date(string_at(node, "currPrdStartDate")).unwrap_or_default(),
            period_end: clean_date(string_at(node, "currPrdEndDate")).unwrap_or_default(),
            reading_date: clean_date(string_at(node, "currRdgDate")).unwrap_or_default(),
            reading: int_at(node, "currRdg"),
            usage: int_at(node, "currUsage"),
        });
    }

    // Priors 1..3 (newest-first in the payload).
    for n in 1..=3 {
        let Some(due) = clean_date(string_at(node, &format!("priorDueDate{n}"))) else {
            continue;
        };
        let amount = float_at(node, &format!("priorPrdBilled{n}"));
        let interest = float_at(node, &format!("priorPrdInt{n}"));
        let principal_balance = float_at(node, &format!("priorPrdPrnBal{n}"));
        out.push(BillPeriod {
            due_date: due,
            service: service.to_string(),
            amount,
            interest,
            principal_balance,
            paid: principal_balance.abs() < f64::EPSILON,
            current: false,
            period_start: String::new(),
            period_end: String::new(),
            reading_date: clean_date(string_at(node, &format!("priorRdgDate{n}")))
                .unwrap_or_default(),
            reading: int_at(node, &format!("priorRdg{n}")),
            usage: int_at(node, &format!("priorUsage{n}")),
        });
    }

    out
}

/// Drop the API's `1000-01-01` "never" sentinel and any blank/space-only value.
fn clean_date(s: String) -> Option<String> {
    let s = s.trim().to_string();
    if s.is_empty() || s == "1000-01-01" {
        None
    } else {
        Some(s)
    }
}

fn string_at(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string()
}

fn float_at(v: &Value, key: &str) -> f64 {
    v.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}

fn int_at(v: &Value, key: &str) -> Option<i64> {
    match v.get(key).and_then(Value::as_i64) {
        Some(0) | None => None,
        Some(n) => Some(n),
    }
}

fn is_zero(x: &f64) -> bool {
    x.abs() < f64::EPSILON
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Synthetic account payload with two services, each with a current + three
    // priors. Shaped like the real `GET /wippUtil/{id}` body.
    fn sample() -> Value {
        json!({
            "id": " 1234567  8",
            "chargeTypes": {
                "Sewer       ": {
                    "currDueDate": "2026-08-12",
                    "currPrdBilled": 79.09,
                    "currPrdPrnBal": 79.09,
                    "currPrdInt": 0.0,
                    "currPrdStartDate": "2026-07-01",
                    "currPrdEndDate": "2026-09-30",
                    "currRdgDate": "1000-01-01",
                    "currRdg": 0,
                    "currUsage": 0,
                    "priorDueDate1": "2026-05-13",
                    "priorDueDate2": "2026-02-11",
                    "priorDueDate3": "2025-11-12",
                    "priorPrdBilled1": 79.09,
                    "priorPrdBilled2": 75.33,
                    "priorPrdBilled3": 75.33,
                    "priorPrdInt1": 0.0,
                    "priorPrdInt2": 0.0,
                    "priorPrdInt3": 0.0,
                    "priorPrdPrnBal1": 0.0,
                    "priorPrdPrnBal2": 0.0,
                    "priorPrdPrnBal3": 0.0,
                    "priorRdgDate1": "1000-01-01",
                    "priorRdg1": 0,
                    "priorUsage1": 0
                },
                "Water       ": {
                    "currDueDate": "2026-08-12",
                    "currPrdBilled": 40.00,
                    "currPrdPrnBal": 40.00,
                    "currPrdInt": 0.0,
                    "currRdg": 84213,
                    "currUsage": 3200,
                    "currRdgDate": "2026-06-15",
                    "priorDueDate1": "2026-05-13",
                    "priorPrdBilled1": 38.50,
                    "priorPrdPrnBal1": 0.0,
                    "priorRdg1": 81013,
                    "priorUsage1": 2900,
                    "priorRdgDate1": "2026-03-15"
                }
            }
        })
    }

    #[test]
    fn list_includes_current_plus_three_priors_per_service_and_sorts_newest_first() {
        let periods = BillPeriod::list_from_account(&sample());
        // Sewer: current + 3 priors = 4; Water: current + 1 prior = 2.
        assert_eq!(periods.len(), 6);
        // Newest first. The two 2026-08-12 rows (Sewer/Water) tie on date and
        // break to Sewer < Water alphabetically.
        assert_eq!(
            periods
                .iter()
                .map(|p| (p.due_date.as_str(), p.service.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("2026-08-12", "Sewer"),
                ("2026-08-12", "Water"),
                ("2026-05-13", "Sewer"),
                ("2026-05-13", "Water"),
                ("2026-02-11", "Sewer"),
                ("2025-11-12", "Sewer"),
            ]
        );
    }

    #[test]
    fn current_flag_and_paid_flag_are_set_correctly() {
        let periods = BillPeriod::list_from_account(&sample());
        let current_sewer = periods
            .iter()
            .find(|p| p.due_date == "2026-08-12" && p.service == "Sewer")
            .expect("current sewer");
        assert!(current_sewer.current);
        assert!(!current_sewer.paid);
        assert_eq!(current_sewer.amount, 79.09);
        assert_eq!(current_sewer.period_start, "2026-07-01");
        assert_eq!(current_sewer.period_end, "2026-09-30");

        let paid_prior = periods
            .iter()
            .find(|p| p.due_date == "2025-11-12" && p.service == "Sewer")
            .expect("prior sewer");
        assert!(!paid_prior.current);
        assert!(paid_prior.paid); // zero outstanding principal → paid off
    }

    #[test]
    fn never_sentinel_dates_are_dropped() {
        let periods = BillPeriod::list_from_account(&sample());
        // The Sewer current-period reading date was 1000-01-01 (no meter).
        let s = periods
            .iter()
            .find(|p| p.due_date == "2026-08-12" && p.service == "Sewer")
            .unwrap();
        assert_eq!(s.reading_date, "");
        assert_eq!(s.reading, None);
        assert_eq!(s.usage, None);
    }

    #[test]
    fn meter_reading_survives_when_metered() {
        let periods = BillPeriod::list_from_account(&sample());
        let w = periods
            .iter()
            .find(|p| p.due_date == "2026-08-12" && p.service == "Water")
            .unwrap();
        assert_eq!(w.reading, Some(84213));
        assert_eq!(w.usage, Some(3200));
        assert_eq!(w.reading_date, "2026-06-15");
    }

    #[test]
    fn find_looks_up_by_due_date() {
        let periods = BillPeriod::list_from_account(&sample());
        assert!(BillPeriod::find(&periods, "2026-05-13").is_some());
        assert!(BillPeriod::find(&periods, "1999-01-01").is_none());
    }

    #[test]
    fn empty_account_produces_no_periods() {
        assert!(BillPeriod::list_from_account(&json!({})).is_empty());
        assert!(BillPeriod::list_from_account(&json!({ "chargeTypes": {} })).is_empty());
    }
}
