//! The `lrfl mcp` tools, offline: `tests/fixtures/` behind the [`Portal`] seam,
//! mapped by the commands' own mappers, checked by cli-common's conformance
//! helper. `tests/mcp_stdio.rs` and `tests/mcp_http.rs` drive the real binary.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use pk_cli_mcp::conformance::{assert_conforms, assert_fits, call, definitions_size};
use pk_cli_mcp::Toolset;
use serde_json::json;

use super::*;

fn fixture(raw: &str) -> Value {
    serde_json::from_str(raw).expect("fixture parses")
}

fn linked(ids: &[&str]) -> Vec<LinkedAccount> {
    ids.iter()
        .map(|id| LinkedAccount {
            account_id: (*id).to_string(),
            account_type: "utility".into(),
            balance_due: None,
        })
        .collect()
}

struct Fixtures {
    default_account: Option<String>,
    login: Result<Option<Vec<LinkedAccount>>, AppError>,
    record: Value,
    payments: Value,
    /// Every read, with its account.
    calls: Rc<RefCell<Vec<String>>>,
}

impl Fixtures {
    fn new() -> Self {
        Self {
            default_account: Some("1234567-0".into()),
            login: Ok(None),
            record: fixture(include_str!("../../tests/fixtures/utility_account.json")),
            payments: fixture(include_str!("../../tests/fixtures/payment_history.json")),
            calls: Rc::default(),
        }
    }

    fn guest() -> Self {
        Self {
            default_account: None,
            ..Self::new()
        }
    }

    fn signed_in(ids: &[&str]) -> Self {
        Self {
            login: Ok(Some(linked(ids))),
            ..Self::guest()
        }
    }

    fn log(&self, what: String) {
        self.calls.borrow_mut().push(what);
    }
}

impl Portal for Fixtures {
    fn default_account(&self) -> Option<String> {
        self.default_account.clone()
    }

    fn linked_accounts(&self) -> Result<Option<Vec<LinkedAccount>>, AppError> {
        self.log("linked".into());
        match &self.login {
            Ok(list) => Ok(list.clone()),
            Err(e) => Err(AppError::Auth(e.to_string())),
        }
    }

    fn utility_account(&self, id: &AccountId) -> Result<Value, AppError> {
        self.log(format!("account {}", id.dashed()));
        Ok(self.record.clone())
    }

    fn payment_history(&self, id: &AccountId, after: &str) -> Result<Value, AppError> {
        self.log(format!("payments {} {after}", id.dashed()));
        Ok(self.payments.clone())
    }
}

fn tools_over(portal: Fixtures) -> UtilityTools<LoxSource<Fixtures>> {
    with_source(LoxSource { portal }, false)
}

fn ok(t: &UtilityTools<LoxSource<Fixtures>>, tool: &str, args: Value) -> Value {
    let result = call(t, tool, args);
    assert_fits(t, tool, &result).clone()
}

fn upstream(t: &UtilityTools<LoxSource<Fixtures>>, tool: &str) {
    let result = call(t, tool, json!({}));
    assert!(result.text().starts_with("upstream:"), "{}", result.text());
}

#[test]
fn the_definitions_conform_and_touch_no_portal() {
    let portal = Fixtures::new();
    let calls = portal.calls.clone();
    let t = tools_over(portal);
    assert_conforms(&t);
    assert!(definitions_size(&t) < 8_000);
    let tools = t.tools();
    let names: Vec<&str> = tools.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "loxahatchee_balance",
            "loxahatchee_bills",
            "loxahatchee_payments",
            "loxahatchee_usage"
        ]
    );
    assert!(calls.borrow().is_empty());
}

#[test]
fn balance_adds_principal_and_interest_and_shows_only_the_end_of_the_number() {
    let t = tools_over(Fixtures::new());
    let out = ok(&t, "loxahatchee_balance", json!({}));
    assert_eq!(out["balance"], "12.84");
    assert_eq!(out["due_date"], "2026-08-01");
    assert_eq!(out["account"], "ending 67-0");
    assert!(out.get("account_id").is_none(), "{out}");

    let out = ok(
        &t,
        "loxahatchee_balance",
        json!({"response_format": "detailed"}),
    );
    assert_eq!(out["account_id"], "1234567-0");
}

#[test]
fn a_default_account_reads_no_login() {
    let portal = Fixtures::new();
    let calls = portal.calls.clone();
    let t = tools_over(portal);
    ok(&t, "loxahatchee_balance", json!({}));
    ok(&t, "loxahatchee_bills", json!({"account": "7654321-0"}));
    assert_eq!(
        *calls.borrow(),
        ["account 1234567-0", "account 7654321-0"],
        "the keychain is one sign-in away from any of these"
    );
}

#[test]
fn a_guest_names_any_account_and_is_told_how_when_it_names_none() {
    let portal = Fixtures::guest();
    let calls = portal.calls.clone();
    let t = tools_over(portal);
    let out = ok(&t, "loxahatchee_balance", json!({"account": "7654321-0"}));
    assert_eq!(out["balance"], "12.84");
    assert!(calls.borrow().contains(&"account 7654321-0".to_string()));

    let result = call(&t, "loxahatchee_balance", json!({}));
    assert!(result.text().starts_with("usage:"), "{}", result.text());
    assert!(result.text().contains("`account`"), "{}", result.text());
}

#[test]
fn a_login_with_one_account_needs_no_naming_and_several_are_listed() {
    let portal = Fixtures::signed_in(&["1234567-0"]);
    let t = tools_over(portal);
    assert_eq!(ok(&t, "loxahatchee_bills", json!({}))["returned"], 3);

    let t = tools_over(Fixtures::signed_in(&["1234567-0", "2345678-0"]));
    let result = call(&t, "loxahatchee_bills", json!({}));
    assert!(result.text().starts_with("usage:"), "{}", result.text());
    assert!(result.text().contains("2 accounts"), "{}", result.text());
    let out = ok(&t, "loxahatchee_bills", json!({"account": "2345678-0"}));
    assert_eq!(out["returned"], 3);
}

#[test]
fn a_failed_sign_in_costs_a_named_account_nothing_and_is_reported_with_none_named() {
    let refused = || Err(AppError::Auth("login failed".into()));
    let t = tools_over(Fixtures {
        login: refused(),
        ..Fixtures::guest()
    });
    let out = ok(&t, "loxahatchee_balance", json!({"account": "7654321-0"}));
    assert_eq!(out["balance"], "12.84");

    let result = call(&t, "loxahatchee_balance", json!({}));
    assert!(result.text().starts_with("auth:"), "{}", result.text());
}

#[test]
fn bills_come_newest_first_with_the_paid_state() {
    let t = tools_over(Fixtures::new());
    let page = ok(&t, "loxahatchee_bills", json!({"limit": 2}));
    assert_eq!(page["returned"], 2);
    assert_eq!(page["items"][0]["date"], "2026-08-01");
    assert_eq!(page["items"][0]["amount"], "12.34");
    assert_eq!(page["items"][0]["paid"], false);
    assert_eq!(page["items"][1]["date"], "2026-05-01");
    assert_eq!(page["items"][1]["paid"], true);
}

#[test]
fn two_services_on_one_due_date_are_one_bill_for_their_total() {
    let mut portal = Fixtures::new();
    portal.record = json!({"chargeTypes": {
        "Sewer ": {"totPrnBal": 1.0, "currDueDate": "2026-08-01", "currPrdBilled": 10.0},
        "Water ": {"totPrnBal": 1.0, "currDueDate": "2026-08-01", "currPrdBilled": 5.5},
    }});
    let t = tools_over(portal);
    let page = ok(&t, "loxahatchee_bills", json!({}));
    assert_eq!(page["returned"], 1);
    assert_eq!(page["items"][0]["amount"], "15.50");
}

#[test]
fn payments_come_newest_first_and_look_back_three_years_unless_asked() {
    let portal = Fixtures::new();
    let calls = portal.calls.clone();
    let t = tools_over(portal);
    let page = ok(&t, "loxahatchee_payments", json!({}));
    assert_eq!(page["returned"], 2);
    assert_eq!(page["items"][0]["date"], "2026-06-20");
    assert_eq!(page["items"][0]["amount"], "34.56");
    assert_eq!(page["items"][1]["date"], "2026-05-20");
    let expected = format!("payments 1234567-0 {}", util::years_ago(3));
    assert!(calls.borrow().contains(&expected), "{:?}", calls.borrow());

    let page = ok(&t, "loxahatchee_payments", json!({"since": "2026-06-01"}));
    assert_eq!(page["returned"], 1);
    assert!(calls
        .borrow()
        .contains(&"payments 1234567-0 2026-06-01".to_string()));
}

#[test]
fn usage_names_the_service_whose_meter_it_is_and_skips_what_has_none() {
    let t = tools_over(Fixtures::new());
    let page = ok(&t, "loxahatchee_usage", json!({}));
    assert_eq!(page["returned"], 2);
    assert_eq!(page["items"][0]["period"], "2026-07-01");
    assert_eq!(page["items"][0]["quantity"], 11.0);
    assert_eq!(page["items"][0]["unit"], "Sewer meter units");
}

#[test]
fn an_answer_the_district_did_not_give_is_an_error_not_zero_or_empty() {
    for record in [
        Value::Null,
        json!({"chargeTypes": {}}),
        json!({"chargeTypes": {"Sewer ": {"renamed": 1}}}),
    ] {
        let mut portal = Fixtures::new();
        portal.record = record;
        upstream(&tools_over(portal), "loxahatchee_balance");
    }
    for record in [
        Value::Null,
        json!({"chargeTypes": {"Sewer ": {"currDueDate": "2026-08-01"}}}),
        json!({"chargeTypes": {"Sewer ": {"priorDueDate1": "2026-05-01"}}}),
    ] {
        let mut portal = Fixtures::new();
        portal.record = record;
        upstream(&tools_over(portal), "loxahatchee_bills");
    }
    let mut portal = Fixtures::new();
    portal.record = Value::Null;
    upstream(&tools_over(portal), "loxahatchee_usage");

    for payments in [
        Value::Null,
        json!({"content": []}),
        json!([{"renamed": 1}]),
        json!([{"amt": 1.0, "paymentDate": " "}]),
    ] {
        let mut portal = Fixtures::new();
        portal.payments = payments;
        upstream(&tools_over(portal), "loxahatchee_payments");
    }
}

#[test]
fn no_payments_and_no_meter_are_empty_lists() {
    let mut portal = Fixtures::new();
    portal.payments = json!([]);
    portal.record = json!({"chargeTypes": {"Sewer ": {"totPrnBal": 0.0}}});
    let t = tools_over(portal);
    assert_eq!(ok(&t, "loxahatchee_payments", json!({}))["returned"], 0);
    assert_eq!(ok(&t, "loxahatchee_usage", json!({}))["returned"], 0);
    assert_eq!(ok(&t, "loxahatchee_bills", json!({}))["returned"], 0);
}

#[test]
fn a_bad_argument_reaches_no_portal() {
    let portal = Fixtures::new();
    let calls = portal.calls.clone();
    let t = tools_over(portal);
    for (tool, args) in [
        ("loxahatchee_payments", json!({"since": "yesterday"})),
        ("loxahatchee_bills", json!({"limit": 0})),
        ("loxahatchee_usage", json!({"until": "2026-13-01"})),
    ] {
        let result = call(&t, tool, args);
        assert!(result.text().starts_with("usage:"), "{}", result.text());
    }
    assert!(calls.borrow().is_empty(), "{:?}", calls.borrow());
}

#[test]
fn a_malformed_account_number_is_a_usage_error() {
    let t = tools_over(Fixtures::guest());
    let result = call(&t, "loxahatchee_balance", json!({"account": "1234567"}));
    assert!(result.text().starts_with("usage:"), "{}", result.text());
}

#[test]
fn a_password_in_the_environment_points_at_where_it_is_set() {
    let refused = || Err(AppError::Auth("login failed".into()));
    let source = |portal| LoxSource { portal };
    let portal = Fixtures {
        login: refused(),
        ..Fixtures::guest()
    };
    let text = call(
        &with_source(source(portal), true),
        "loxahatchee_bills",
        json!({}),
    )
    .text()
    .to_string();
    assert!(text.contains("$LRFL_PASSWORD"), "{text}");
    assert!(!text.contains("lrfl auth login"), "{text}");

    let portal = Fixtures {
        login: refused(),
        ..Fixtures::guest()
    };
    let text = call(
        &with_source(source(portal), false),
        "loxahatchee_bills",
        json!({}),
    )
    .text()
    .to_string();
    assert!(text.contains("`lrfl auth login`"), "{text}");
}

mod sign_in {
    use super::*;

    /// A fetch that counts its calls and answers as told.
    fn fetch_with<'a>(
        calls: &'a Cell<u32>,
        answer: impl Fn() -> Result<Vec<LinkedAccount>, AppError> + 'a,
    ) -> impl Fn(&Secret) -> Result<Vec<LinkedAccount>, AppError> + 'a {
        move |_| {
            calls.set(calls.get() + 1);
            answer()
        }
    }

    fn stored(value: &'static str) -> impl Fn() -> Result<Secret, AppError> {
        move || Ok(Secret::new(value))
    }

    fn refuse() -> Result<Vec<LinkedAccount>, AppError> {
        Err(AppError::Auth("login failed".into()))
    }

    fn one() -> Result<Vec<LinkedAccount>, AppError> {
        Ok(linked(&["1234567-0"]))
    }

    #[test]
    fn a_refused_password_is_not_sent_again_and_a_different_one_is() {
        let calls = Cell::new(0);
        let now = Instant::now();
        let mut state = Linked::default();
        let fetch = fetch_with(&calls, refuse);
        assert!(state.read("user", now, stored("old"), &fetch).is_err());
        let again = state.read("user", now, stored("old"), &fetch).unwrap_err();
        assert_eq!(calls.get(), 1, "the refused password went out again");
        assert!(again.to_string().contains("not sent again"), "{again}");

        let fetch = fetch_with(&calls, one);
        assert_eq!(
            state
                .read("user", now, stored("new"), &fetch)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn the_same_password_under_a_new_login_is_sent_once() {
        let calls = Cell::new(0);
        let now = Instant::now();
        let mut state = Linked::default();
        let fetch = fetch_with(&calls, refuse);
        assert!(state.read("mistyped", now, stored("p"), &fetch).is_err());
        assert!(state.read("mistyped", now, stored("p"), &fetch).is_err());
        assert_eq!(calls.get(), 1);

        let fetch = fetch_with(&calls, one);
        assert!(state.read("right", now, stored("p"), &fetch).is_ok());
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn the_district_being_down_is_not_a_refusal() {
        let calls = Cell::new(0);
        let now = Instant::now();
        let mut state = Linked::default();
        let fetch = fetch_with(&calls, || Err(AppError::Upstream("HTTP 503".into())));
        assert!(state.read("user", now, stored("p"), &fetch).is_err());
        assert!(state.read("user", now, stored("p"), &fetch).is_err());
        assert_eq!(calls.get(), 2, "an outage must not stop the next try");
    }

    #[test]
    fn the_list_is_kept_for_its_login_for_an_hour() {
        let calls = Cell::new(0);
        let now = Instant::now();
        let mut state = Linked::default();
        let fetch = fetch_with(&calls, one);
        let unread = || -> Result<Secret, AppError> { panic!("the keychain was read") };
        state.read("user", now, stored("p"), &fetch).unwrap();
        state.read("user", now, unread, &fetch).unwrap();
        let before_the_hour = now + KEEP_LINKED - Duration::from_secs(1);
        state.read("user", before_the_hour, unread, &fetch).unwrap();
        assert_eq!(calls.get(), 1);

        state
            .read("user", now + KEEP_LINKED, stored("p"), &fetch)
            .unwrap();
        assert_eq!(calls.get(), 2);

        state
            .read("other", now + KEEP_LINKED, stored("p"), &fetch)
            .unwrap();
        assert_eq!(calls.get(), 3);
    }

    #[test]
    fn a_token_turned_away_after_a_good_login_is_not_a_refusal() {
        assert!(matches!(
            not_a_refusal(AppError::Auth("session expired".into())),
            AppError::Upstream(_)
        ));
        assert!(matches!(
            not_a_refusal(AppError::NotFound("gone".into())),
            AppError::NotFound(_)
        ));
    }
}

/// Every identity-bearing value in the fixtures is a placeholder.
#[test]
fn the_fixtures_hold_only_placeholder_identifiers() {
    let record = fixture(include_str!("../../tests/fixtures/utility_account.json"));
    assert_eq!(record["id"], "1234567 0");
    assert_eq!(record["billToName"], "EXAMPLE OWNER");
    assert!(record["serviceLoc"]
        .as_str()
        .unwrap()
        .ends_with("EXAMPLE STREET"));
    let payments = fixture(include_str!("../../tests/fixtures/payment_history.json"));
    for row in payments.as_array().unwrap() {
        assert!(row["transactionId"]
            .as_str()
            .unwrap()
            .starts_with("CONF000"));
    }
}
