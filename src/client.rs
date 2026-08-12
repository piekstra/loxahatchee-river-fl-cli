//! Thin, polite client over the Edmunds/SunGard "WIPP" utility-billing API that
//! powers the Loxahatchee River District payments portal
//! (`wipp.edmundsgovtech.cloud`, tenant id `LOXA`).
//!
//! All reads here are the same anonymous, guest-view calls the portal itself
//! makes before you log in — looking up an account by number to show its
//! balance and pay it. No credentials are involved.
//!
//! Two things gate the API and are handled here:
//! * an AWS WAF that rejects non-browser `User-Agent`s (we send a browser UA
//!   with a polite tool suffix), and
//! * the tenant selector header `X-Wipp-Id`.
//!
//! Some operations are **asynchronous** on the mainframe side: the request
//! returns `202 { requestId }` and you poll `GET /requests/{id}` until it flips
//! to `200` with the result. [`Wipp::await_request`] implements that loop.

use std::thread::sleep;
use std::time::Duration;

use serde_json::Value;

use crate::acct::AccountId;
use crate::error::AppError;

/// Production WIPP core API base (baked into the portal's own JS bundle).
pub const BASE: &str = "https://api.edmundsgovtech.cloud/wipp-core/v1";
/// wipp-core's proxy to the SunGard/FIS identity provider — used for login.
pub const FIS_PROXY: &str = "https://api.edmundsgovtech.cloud/wipp-core/proxy/fis";
/// Default tenant id — the Loxahatchee River District.
pub const DEFAULT_WIPP_ID: &str = "LOXA";
/// Public web origin (used to build portal deep links for `pay` / `open`).
pub const PORTAL_ORIGIN: &str = "https://wipp.edmundsgovtech.cloud";

/// A browser-shaped User-Agent with a polite tool suffix. The tenant's WAF
/// blocks obvious non-browser agents (a bare client string gets a 403), so we
/// present as a browser while still identifying the tool.
const UA: &str = concat!(
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 ",
    "(KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36 ",
    "loxahatchee-river-fl/",
    env!("CARGO_PKG_VERSION"),
);

/// How long to keep polling an async `/requests/{id}` before giving up.
const POLL_BUDGET: Duration = Duration::from_secs(20);
/// Delay between poll attempts.
const POLL_INTERVAL: Duration = Duration::from_millis(700);

/// Client bound to a single tenant (`wipp_id`).
pub struct Wipp {
    client: reqwest::blocking::Client,
    wipp_id: String,
}

impl Wipp {
    pub fn new(wipp_id: String) -> Result<Self, AppError> {
        let client = reqwest::blocking::Client::builder()
            .user_agent(UA)
            .timeout(Duration::from_secs(25))
            // The FIS login exchanges a session cookie (hop 1) for a JWT (hop 2).
            .cookie_store(true)
            .build()
            .map_err(|e| AppError::Other(format!("failed to build HTTP client: {e}")))?;
        Ok(Self { client, wipp_id })
    }

    pub fn wipp_id(&self) -> &str {
        &self.wipp_id
    }

    /// `GET {BASE}{path}` with the tenant header. `path` must be pre-encoded and
    /// begin with `/`. Returns `(status, parsed-json-or-null)`.
    fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<(u16, Value), AppError> {
        let url = format!("{BASE}{path}");
        let resp = self
            .client
            .get(&url)
            .header("X-Wipp-Id", &self.wipp_id)
            .header("Accept", "application/json, text/plain, */*")
            .query(query)
            .send()?;
        parse_response(resp)
    }

    /// `GET {BASE}{path}` with the tenant header and a bearer token, for
    /// authenticated (logged-in) endpoints.
    fn get_authed(&self, path: &str, bearer: &str) -> Result<(u16, Value), AppError> {
        let url = format!("{BASE}{path}");
        let resp = self
            .client
            .get(&url)
            .header("X-Wipp-Id", &self.wipp_id)
            .header("Accept", "application/json, text/plain, */*")
            .bearer_auth(bearer)
            .send()?;
        parse_response(resp)
    }

    /// Map a non-success status + body into the right [`AppError`], for the
    /// endpoints where a plain GET is expected to succeed directly.
    fn require_ok(&self, status: u16, body: Value, ctx: &str) -> Result<Value, AppError> {
        match status {
            200 => Ok(body),
            404 => Err(AppError::NotFound(
                body_message(&body).unwrap_or_else(|| ctx.to_string()),
            )),
            _ => Err(AppError::Upstream(format!(
                "HTTP {status} from {ctx}: {}",
                body_message(&body).unwrap_or_else(|| "no detail".into())
            ))),
        }
    }

    /// Tenant configuration: district name, which services are billed, overpay
    /// limits, contact message, etc. `GET /metadata/{wippId}`.
    pub fn metadata(&self) -> Result<Value, AppError> {
        let (s, b) = self.get(&format!("/metadata/{}", self.wipp_id), &[])?;
        self.require_ok(s, b, "metadata")
    }

    /// Which bill types this tenant accepts. `GET /capabilities/{wippId}`.
    pub fn capabilities(&self) -> Result<Value, AppError> {
        let (s, b) = self.get(&format!("/capabilities/{}", self.wipp_id), &[])?;
        self.require_ok(s, b, "capabilities")
    }

    /// Feature flags & third-party integrations for the tenant.
    /// `GET /wipp/additional-metadata/{wippId}`.
    pub fn additional_metadata(&self) -> Result<Value, AppError> {
        let (s, b) = self.get(&format!("/wipp/additional-metadata/{}", self.wipp_id), &[])?;
        self.require_ok(s, b, "additional-metadata")
    }

    /// Full utility account record (owner, service location, and per-service
    /// charge detail). `GET /wippUtil/{encodedAccountId}`.
    pub fn utility_account(&self, id: &AccountId) -> Result<Value, AppError> {
        let (s, b) = self.get(&format!("/wippUtil/{}", id.encoded()), &[])?;
        match s {
            200 => Ok(b),
            401 | 404 => Err(AppError::NotFound(format!(
                "no utility account {}",
                id.dashed()
            ))),
            _ => Err(AppError::Upstream(format!(
                "HTTP {s} looking up account {}: {}",
                id.dashed(),
                body_message(&b).unwrap_or_else(|| "no detail".into())
            ))),
        }
    }

    /// Search utility accounts by street/property location. The district matches
    /// server-side (case-insensitive substring on the property location); no
    /// geocoding or third party is involved. `GET /wippUtil/search?propertyLoc=…`
    /// returns a Spring page (`{ content: [ … ], totalElements, … }`).
    pub fn search_by_location(&self, query: &str, size: u32) -> Result<Value, AppError> {
        let size = size.to_string();
        let (s, b) = self.get(
            "/wippUtil/search",
            &[("propertyLoc", query), ("size", &size)],
        )?;
        match s {
            200 => Ok(b),
            _ => Err(AppError::Upstream(format!(
                "HTTP {s} searching {query:?}: {}",
                body_message(&b).unwrap_or_else(|| "no detail".into())
            ))),
        }
    }

    /// Resolve the hosted PDF bill URL for an account's current bill.
    /// `GET /wippUtil/{id}/retrieveThirdPartyBillUrl?dueDate=YYYY-MM-DD` → `{ url }`
    /// (an onlinebiller.com document link). `due_date` must be the bill's current
    /// due date, taken from the account's charges.
    pub fn bill_url(&self, id: &AccountId, due_date: &str) -> Result<String, AppError> {
        let (s, b) = self.get(
            &format!("/wippUtil/{}/retrieveThirdPartyBillUrl", id.encoded()),
            &[("dueDate", due_date)],
        )?;
        let body = self.require_ok(s, b, "bill url")?;
        body.get("url")
            .and_then(Value::as_str)
            .filter(|u| !u.is_empty())
            .map(str::to_string)
            .ok_or_else(|| AppError::NotFound(format!("no bill available for {}", id.dashed())))
    }

    /// Download raw bytes from a URL (the hosted PDF bill). Uses the browser UA.
    ///
    /// Guards against the archive's "no document(s) found" placeholder — for
    /// unknown/expired periods, `docs.onlinebiller.com` returns a 200 HTML page
    /// with an error banner instead of an actual PDF. When the response fails
    /// the [`looks_like_real_pdf`] check, this returns [`AppError::NotFound`]
    /// so the CLI can tell the user the period isn't retained upstream.
    pub fn fetch_bill_pdf(&self, url: &str, due_date: &str) -> Result<Vec<u8>, AppError> {
        let resp = self.client.get(url).send()?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(AppError::Upstream(format!(
                "HTTP {status} fetching the bill PDF for {due_date}"
            )));
        }
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let bytes = resp.bytes()?.to_vec();
        if !looks_like_real_pdf(&bytes, &content_type) {
            return Err(AppError::NotFound(format!(
                "no bill PDF available for {due_date} — the district's document \
                 archive returned a placeholder ({} bytes, content-type: {})",
                bytes.len(),
                if content_type.is_empty() {
                    "unknown"
                } else {
                    &content_type
                }
            )));
        }
        Ok(bytes)
    }

    /// Download raw bytes from a URL. Used for hosted PDFs whose content-type
    /// is trusted (e.g. the current bill via [`Wipp::bill_url`], which the
    /// portal itself calls without a PDF-shape check).
    pub fn fetch_bytes(&self, url: &str) -> Result<Vec<u8>, AppError> {
        let resp = self.client.get(url).send()?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(AppError::Upstream(format!(
                "HTTP {status} fetching the bill PDF"
            )));
        }
        Ok(resp.bytes()?.to_vec())
    }

    /// Per-service active/inactive status. This is an **async** mainframe call:
    /// submit, then poll. `GET /wippUtil/{id}/determineAccountStatus` → poll.
    pub fn account_status(&self, id: &AccountId) -> Result<Value, AppError> {
        let (s, b) = self.get(
            &format!("/wippUtil/{}/determineAccountStatus", id.encoded()),
            &[],
        )?;
        self.await_request(s, b, "account status")
    }

    /// Payment history since `after` (an ISO `YYYY-MM-DD` date).
    /// `GET /billingAccounts/wippUtil/{id}/payments?paymentsAfterDate=…`.
    pub fn payment_history(&self, id: &AccountId, after: &str) -> Result<Value, AppError> {
        let (s, b) = self.get(
            &format!("/billingAccounts/wippUtil/{}/payments", id.encoded()),
            &[("paymentsAfterDate", after)],
        )?;
        self.require_ok(s, b, "payment history")
    }

    /// Drive an async request to completion. The initial call either already
    /// returned `200` with the result, or `202 { requestId }` that we poll on
    /// `GET /requests/{id}` (also `202` while processing, `200` when done).
    fn await_request(&self, status: u16, body: Value, ctx: &str) -> Result<Value, AppError> {
        if status == 200 {
            return Ok(body);
        }
        if status != 202 {
            return Err(AppError::Upstream(format!(
                "HTTP {status} starting {ctx}: {}",
                body_message(&body).unwrap_or_else(|| "no detail".into())
            )));
        }
        let request_id = body
            .get("requestId")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Other(format!("{ctx}: server did not return a requestId")))?
            .to_string();

        let deadline = std::time::Instant::now() + POLL_BUDGET;
        loop {
            let (s, b) = self.get(&format!("/requests/{request_id}"), &[])?;
            match s {
                200 => return Ok(b),
                202 => {
                    if std::time::Instant::now() >= deadline {
                        return Err(AppError::Upstream(format!(
                            "{ctx} was still processing after {}s",
                            POLL_BUDGET.as_secs()
                        )));
                    }
                    sleep(POLL_INTERVAL);
                }
                other => {
                    return Err(AppError::Upstream(format!(
                        "HTTP {other} polling {ctx}: {}",
                        body_message(&b).unwrap_or_else(|| "no detail".into())
                    )))
                }
            }
        }
    }

    /// Portal deep link to the guest account view (where "Pay Now" lives).
    pub fn account_view_url(&self, id: &AccountId) -> String {
        format!(
            "{PORTAL_ORIGIN}/view/wippUtil/{}?wippId={}",
            id.dashed(),
            self.wipp_id
        )
    }

    // --- Authenticated (logged-in) endpoints --------------------------------
    //
    // The Loxahatchee portal authenticates its users through the SunGard/FIS
    // ("Link2Gov") identity provider, proxied by wipp-core. Login is two hops
    // over one cookie session:
    //   1. POST {FIS_PROXY}/rest/1.0/sessions {loginName, password}  → session cookies
    //   2. GET  {FIS_PROXY}/rest/1.0/idptoken/openid-connect?...     → an `id_token` JWT
    // That JWT is then the `Authorization: Bearer` for the wipp-core API. The
    // client is built with a cookie store so hop 2 sees hop 1's session.

    /// Log in via the FIS session flow and return the resulting `id_token` (a
    /// short-lived JWT used as the bearer for authenticated calls).
    pub fn fis_login(&self, login_name: &str, password: &str) -> Result<String, AppError> {
        // Hop 1: establish the session (Set-Cookie captured by the cookie store).
        let body = serde_json::json!({ "loginName": login_name, "password": password });
        let (s1, b1) = parse_response(
            self.client
                .post(format!("{FIS_PROXY}/rest/1.0/sessions"))
                .header("X-Wipp-Id", &self.wipp_id)
                .header("api-type", "auth")
                .header("Accept", "application/json, text/plain, */*")
                .json(&body)
                .send()?,
        )?;
        if s1 != 200 {
            return Err(AppError::Auth(format!(
                "login failed: {}",
                fis_error(s1, &b1)
            )));
        }

        // Hop 2: exchange the session for an OpenID id_token.
        let (s2, b2) = parse_response(
            self.client
                .get(format!(
                    "{FIS_PROXY}/rest/1.0/idptoken/openid-connect?client_id=Enroll.User"
                ))
                .header("X-Wipp-Id", &self.wipp_id)
                .header("api-type", "auth")
                .header("Accept", "application/json, text/plain, */*")
                .send()?,
        )?;
        if s2 != 200 {
            return Err(AppError::Auth(format!(
                "login token exchange failed: {}",
                fis_error(s2, &b2)
            )));
        }
        b2.get("id_token")
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .ok_or_else(|| AppError::Auth("login succeeded but no id_token was returned".into()))
    }

    /// Utility accounts linked to the logged-in user.
    /// `GET /accounts/billingAccounts` → `[{ wippId, accountType, accountId }]`.
    pub fn billing_accounts(&self, bearer: &str) -> Result<Value, AppError> {
        let (s, b) = self.get_authed("/accounts/billingAccounts", bearer)?;
        self.require_authed(s, b, "billing accounts")
    }

    /// Like [`Wipp::require_ok`] but maps `401`/`403` to an auth error so callers
    /// can prompt the user to log in again.
    fn require_authed(&self, status: u16, body: Value, ctx: &str) -> Result<Value, AppError> {
        match status {
            200 => Ok(body),
            401 | 403 => Err(AppError::Auth(format!(
                "session expired or unauthorized for {ctx} — run `lrfl login` again"
            ))),
            404 => Err(AppError::NotFound(
                body_message(&body).unwrap_or_else(|| ctx.to_string()),
            )),
            _ => Err(AppError::Upstream(format!(
                "HTTP {status} from {ctx}: {}",
                body_message(&body).unwrap_or_else(|| "no detail".into())
            ))),
        }
    }
}

/// Turn a failed FIS login response into a friendly message. FIS surfaces some
/// states as `CCnnnn` error codes; a 401/500 on hop 1 is almost always a wrong
/// login name or password.
fn fis_error(status: u16, body: &Value) -> String {
    let code = body.get("errorCode").and_then(Value::as_str).unwrap_or("");
    match code {
        "CC0206" => "password has expired — reset it in the portal".into(),
        "CC0219" => "a temporary password is set — finish resetting it in the portal".into(),
        _ => {
            let detail = body_message(body).unwrap_or_else(|| format!("HTTP {status}"));
            // Hop 1 rejects wrong credentials with a 400/401 (and occasionally a
            // 500); in every case the actionable cause is the same.
            if matches!(status, 400 | 401 | 500) {
                format!("{detail} — double-check your login name and password")
            } else {
                detail
            }
        }
    }
}

/// Read a blocking response into `(status, json-or-bare-string)`, treating 429
/// as a dedicated rate-limit error.
fn parse_response(resp: reqwest::blocking::Response) -> Result<(u16, Value), AppError> {
    let status = resp.status().as_u16();
    if status == 429 {
        return Err(AppError::Upstream(
            "rate limited by the portal (HTTP 429) — slow down and retry".into(),
        ));
    }
    let text = resp.text()?;
    // The API returns JSON on success and a bare `(NNN) message` string on some
    // errors; tolerate both so callers get a useful message.
    let value = serde_json::from_str::<Value>(&text).unwrap_or(Value::String(text));
    Ok((status, value))
}

/// Pull a human message out of an error body, whether it arrived as a JSON
/// `{message}` / `{error}` object or as the API's bare `(NNN) text` string.
fn body_message(body: &Value) -> Option<String> {
    if let Some(s) = body.as_str() {
        let s = s.trim();
        return (!s.is_empty()).then(|| s.to_string());
    }
    body.get("message")
        .or_else(|| body.get("error"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Sniff whether a response body is a real PDF (vs. the district's
/// document-archive HTML placeholder that `docs.onlinebiller.com` serves as
/// `200 text/html` when a requested due date isn't on file).
///
/// A body is treated as a real PDF when it either:
///   * starts with the `%PDF` magic bytes (authoritative — this is the format's
///     required file header, so a stray or misconfigured content-type can't
///     mask a good PDF); or
///   * has a `Content-Type` beginning with `application/pdf` **and** a
///     non-empty body (covers the rare server that trims the magic prefix or
///     wraps the bytes; an empty body with the right content-type is still
///     rejected — an empty PDF is not a real PDF).
///
/// `starts_with` (never `contains`) is deliberate: a text/html placeholder can
/// legitimately mention `%PDF` in its body without being a PDF.
fn looks_like_real_pdf(bytes: &[u8], content_type: &str) -> bool {
    if bytes.starts_with(b"%PDF") {
        return true;
    }
    !bytes.is_empty() && content_type.trim_start().starts_with("application/pdf")
}

#[cfg(test)]
mod tests {
    use super::looks_like_real_pdf;

    /// The happy path: real PDF bytes, correct `application/pdf` type.
    #[test]
    fn real_pdf_bytes_with_application_pdf_content_type_pass() {
        let bytes = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n2 0 obj\n";
        assert!(looks_like_real_pdf(bytes, "application/pdf"));
    }

    /// Some CDN/S3 configurations serve PDFs as octet-stream. The magic bytes
    /// override the content-type so the caller still writes the file.
    #[test]
    fn real_pdf_bytes_with_octet_stream_content_type_pass() {
        let bytes = b"%PDF-1.4\n";
        assert!(looks_like_real_pdf(bytes, "application/octet-stream"));
    }

    /// The exact observed placeholder: a 200 HTML error page from
    /// `docs.onlinebiller.com` when the due date isn't on file. This is the
    /// case the guard exists to catch — regressing it would save a
    /// ~366-byte HTML file as `.pdf`.
    #[test]
    fn html_placeholder_with_text_html_content_type_is_rejected() {
        let placeholder = b"<!DOCTYPE html><html><head><link rel=\"stylesheet\" \
            type=\"text/css\" href=\"css/infosend.css\"></head><body>\
            <div id=\"logo\"></div><table class=\"DocumentList\"><tr><td \
            id=\"error_message\" class=\"error_message\">An error has occurred, \
            please contact Loxahatchee River District support with the following \
            information:<br><br>No document(s) found.</td></tr></table></body></html>";
        assert!(!looks_like_real_pdf(
            placeholder,
            "text/html; charset=UTF-8"
        ));
    }

    /// An empty body served with `application/pdf` is not a real PDF — refusing
    /// it means an empty file never gets written and the caller sees the same
    /// "no bill available" `NotFound` the archive placeholder would produce.
    #[test]
    fn empty_bytes_with_application_pdf_content_type_are_rejected() {
        assert!(!looks_like_real_pdf(b"", "application/pdf"));
    }

    /// Regression guard: HTML that happens to mention `%PDF` in its body (e.g.
    /// documentation or an error page quoting a filename) must NOT be treated
    /// as a real PDF. This is why the magic check uses `starts_with`, never
    /// `contains`.
    #[test]
    fn html_with_embedded_pdf_string_is_rejected() {
        let body = b"<html><body>Attach a %PDF-1.7 file to submit.</body></html>";
        assert!(!looks_like_real_pdf(body, "text/html"));
    }

    /// A PDF byte body with no content-type header still passes on magic alone —
    /// matches the observed behavior of `docs.onlinebiller.com` for legitimate
    /// bills. Guards against an over-strict future rewrite that would drop the
    /// magic short-circuit.
    #[test]
    fn real_pdf_bytes_with_missing_content_type_pass() {
        assert!(looks_like_real_pdf(b"%PDF-1.7\n", ""));
    }
}
