//! Normalized models for a utility account and its charges.
//!
//! The raw `GET /wippUtil/{id}` payload is a wide, mainframe-flavored object
//! (dozens of `prior*1/2/3` columns, space-padded keys, sentinel blanks). We
//! distill it into a few stable structs and compute the amount due exactly the
//! way the portal's own front end does.

use serde::Serialize;
use serde_json::Value;

/// The account owner / bill-to party. Always rendered — the CLI shows whatever
/// the portal returns for the account (there is no redaction flag).
#[derive(Debug, Clone, Serialize, Default)]
pub struct Owner {
    pub name: String,
    pub street1: String,
    pub street2: String,
    pub city_state: String,
    pub zip: String,
}

/// One billed service (Water, Sewer, Electric, Other) on the account.
#[derive(Debug, Clone, Serialize)]
pub struct ServiceCharge {
    /// Service name, trimmed (the API keys are space-padded, e.g. `"Sewer "`).
    pub service: String,
    /// Amount due for this service: principal + interest, less any not-yet-due
    /// future principal. Mirrors the portal's account-balance math.
    pub amount_due: f64,
    pub total_principal: f64,
    pub total_interest: f64,
    pub future_principal: f64,
    pub current_due_date: String,
    pub last_paid_date: String,
    pub billed_ytd: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_reading: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_usage: Option<i64>,
    #[serde(skip_serializing_if = "str::is_empty")]
    pub current_reading_date: String,
    #[serde(skip_serializing_if = "str::is_empty")]
    pub current_period_start: String,
    #[serde(skip_serializing_if = "str::is_empty")]
    pub current_period_end: String,
    /// Early-payment discount, if the tenant offers one on this service.
    #[serde(skip_serializing_if = "is_zero")]
    pub discount_amount: f64,
    #[serde(skip_serializing_if = "str::is_empty")]
    pub discount_date: String,
}

/// A distilled utility account.
#[derive(Debug, Clone, Serialize)]
pub struct Account {
    pub id: String,
    pub bill_to_name: String,
    pub owner: Owner,
    pub service_location: String,
    pub property_location: String,
    /// Date through which interest is calculated (`propertyInfo.interestDate`).
    /// The mainframe's `1000-01-01` "never" sentinel is normalized to blank.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub interest_date: String,
    pub charges: Vec<ServiceCharge>,
    /// Total amount due across all services.
    pub balance_due: f64,
}

/// Whether a `GET /wippUtil/{id}` body states what the balance is built from.
/// [`Account::from_node`] reads a missing principal as 0.00, and no services as
/// nothing owed.
pub fn balance_stated(v: &Value) -> bool {
    v.get("chargeTypes")
        .and_then(Value::as_object)
        .is_some_and(|services| {
            !services.is_empty()
                && services
                    .values()
                    .all(|node| node.get("totPrnBal").is_some_and(Value::is_number))
        })
}

impl Account {
    /// Build an [`Account`] from a `GET /wippUtil/{id}` body. `display_id` is the
    /// caller's dashed account number (the API echoes a space-padded form).
    pub fn from_node(display_id: &str, v: &Value) -> Account {
        let owner = v
            .get("utilityOwnerInfo")
            .map(Owner::from_node)
            .unwrap_or_default();
        let mut charges: Vec<ServiceCharge> = v
            .get("chargeTypes")
            .and_then(Value::as_object)
            .map(|m| {
                m.iter()
                    .map(|(name, node)| ServiceCharge::from_node(name, node))
                    .collect()
            })
            .unwrap_or_default();
        charges.sort_by(|a, b| a.service.cmp(&b.service));
        let balance_due = round2(charges.iter().map(|c| c.amount_due).sum());

        // Interest-through date lives on the nested propertyInfo block; drop the
        // mainframe "never" sentinel.
        let interest_date = v
            .get("propertyInfo")
            .map(|p| string_at(p, "interestDate"))
            .filter(|d| d != "1000-01-01")
            .unwrap_or_default();

        Account {
            id: display_id.to_string(),
            bill_to_name: string_at(v, "billToName"),
            owner,
            service_location: string_at(v, "serviceLoc"),
            property_location: string_at(v, "propertyLoc"),
            interest_date,
            charges,
            balance_due,
        }
    }
}

impl Owner {
    fn from_node(v: &Value) -> Owner {
        Owner {
            name: string_at(v, "name"),
            street1: string_at(v, "street1"),
            street2: string_at(v, "street2"),
            city_state: string_at(v, "cityState"),
            zip: string_at(v, "zip"),
        }
    }

    /// A one-line address (blank sentinel parts dropped).
    pub fn address_line(&self) -> String {
        [&self.street1, &self.street2, &self.city_state, &self.zip]
            .into_iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl ServiceCharge {
    fn from_node(name: &str, v: &Value) -> ServiceCharge {
        let total_principal = float_at(v, "totPrnBal");
        let total_interest = float_at(v, "totIntDue");
        let future_principal = float_at(v, "futurePrnBal");
        // Portal account-balance math: principal + interest − future principal.
        let amount_due = round2(total_principal + total_interest - future_principal);
        ServiceCharge {
            service: name.trim().to_string(),
            amount_due,
            total_principal,
            total_interest,
            future_principal,
            current_due_date: string_at(v, "currDueDate"),
            last_paid_date: string_at(v, "lastDatePaid"),
            billed_ytd: float_at(v, "billedYtd"),
            current_reading: int_at(v, "currRdg"),
            current_usage: int_at(v, "currUsage"),
            current_reading_date: string_at(v, "currRdgDate"),
            current_period_start: string_at(v, "currPrdStartDate"),
            current_period_end: string_at(v, "currPrdEndDate"),
            discount_amount: float_at(v, "currDiscAmt"),
            discount_date: string_at(v, "currDiscDate"),
        }
    }
}

/// Trim a string field; the mainframe uses a single space as a "blank" sentinel.
fn string_at(v: &Value, key: &str) -> String {
    let s = v.get(key).and_then(Value::as_str).unwrap_or("").trim();
    s.to_string()
}

fn float_at(v: &Value, key: &str) -> f64 {
    v.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}

/// Read an integer field, treating the mainframe's `0` reading/usage sentinels
/// as "no data".
fn int_at(v: &Value, key: &str) -> Option<i64> {
    match v.get(key).and_then(Value::as_i64) {
        Some(0) | None => None,
        Some(n) => Some(n),
    }
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

fn is_zero(x: &f64) -> bool {
    x.abs() < f64::EPSILON
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Synthetic payload — shaped like the real API, no real account data.
    fn sample() -> Value {
        json!({
            "id": " 1234567  8",
            "billToName": "PUBLIC, JOHN Q",
            "utilityOwnerInfo": {
                "name": "PUBLIC, JOHN Q",
                "street1": "123 MAIN ST",
                "street2": " ",
                "cityState": "ANYTOWN FL",
                "zip": "33400"
            },
            "serviceLoc": "123 MAIN ST",
            "propertyLoc": "123 MAIN ST",
            "propertyInfo": { "interestDate": "2026-07-20" },
            "chargeTypes": {
                "Sewer       ": {
                    "totPrnBal": 79.09,
                    "totIntDue": 0.0,
                    "futurePrnBal": 0.0,
                    "billedYtd": 158.18,
                    "currDueDate": "2026-07-01",
                    "lastDatePaid": "2026-04-15",
                    "currRdg": 0,
                    "currUsage": 0,
                    "currRdgDate": " ",
                    "currPrdStartDate": "2026-04-01",
                    "currPrdEndDate": "2026-06-30",
                    "currDiscAmt": 0.0,
                    "currDiscDate": " "
                },
                "Water       ": {
                    "totPrnBal": 20.0,
                    "totIntDue": 1.50,
                    "futurePrnBal": 5.0,
                    "billedYtd": 60.0,
                    "currDueDate": "2026-07-01",
                    "lastDatePaid": "2026-04-15",
                    "currRdg": 84213,
                    "currUsage": 3200,
                    "currRdgDate": "2026-06-15"
                }
            }
        })
    }

    #[test]
    fn balance_is_principal_plus_interest_minus_future() {
        let acct = Account::from_node("1234567-8", &sample());
        // Sewer: 79.09; Water: 20 + 1.50 − 5 = 16.50 → total 95.59
        assert_eq!(acct.balance_due, 95.59);
    }

    #[test]
    fn charges_are_trimmed_and_sorted() {
        let acct = Account::from_node("1234567-8", &sample());
        assert_eq!(acct.charges.len(), 2);
        assert_eq!(acct.charges[0].service, "Sewer");
        assert_eq!(acct.charges[1].service, "Water");
    }

    #[test]
    fn zero_readings_are_treated_as_absent() {
        let acct = Account::from_node("1234567-8", &sample());
        let sewer = &acct.charges[0];
        assert_eq!(sewer.current_reading, None);
        assert_eq!(sewer.current_usage, None);
        let water = &acct.charges[1];
        assert_eq!(water.current_reading, Some(84213));
        assert_eq!(water.current_usage, Some(3200));
    }

    #[test]
    fn blank_sentinels_are_stripped() {
        let acct = Account::from_node("1234567-8", &sample());
        assert_eq!(acct.owner.street2, "");
        assert_eq!(acct.owner.address_line(), "123 MAIN ST, ANYTOWN FL, 33400");
    }

    #[test]
    fn interest_date_read_from_property_info() {
        let acct = Account::from_node("1234567-8", &sample());
        assert_eq!(acct.interest_date, "2026-07-20");
    }

    #[test]
    fn interest_date_never_sentinel_is_blank() {
        let mut v = sample();
        v["propertyInfo"]["interestDate"] = json!("1000-01-01");
        let acct = Account::from_node("1234567-8", &v);
        assert_eq!(acct.interest_date, "");
    }
}
