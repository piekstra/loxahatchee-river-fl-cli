# CLAUDE.md — loxahatchee-river-fl

Guidance for AI agents working in this repo.

## What this is

A Rust CLI (`lrfl`) over the **WIPP** (Edmunds GovTech / SunGard) utility-billing
API that powers the Loxahatchee River District payment portal (tenant `LOXA`).
Guest commands are anonymous **guest-view** lookups by account number — no
credentials. Authenticated commands (`login`/`logout`/`whoami`, `accounts`) use
the district's **SunGard/FIS** login (a two-hop cookie→JWT flow); the password is
stored in the OS keychain and a fresh token is minted per command. `lrfl pay`
computes what's owed and hands off to the portal's Pay Now page — reCAPTCHA and a
processor-hosted card form leave no programmatic path — rather than touching card
data. `lrfl self-update` pulls the latest GitHub release.

Structured as a **library + thin binary** (like the pup CLI): logic lives in the
`loxahatchee_river_fl` lib (`src/lib.rs`); `main.rs` only parses args and
dispatches into `src/commands/`.

## Build / test / run

```sh
cargo build --release
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all

./target/release/lrfl balance 1234567-0
./target/release/lrfl district --json
```

## Layout

- `src/lib.rs` — library root (declares the modules below).
- `src/main.rs` — thin binary: parse args, build `Ctx`, dispatch to `commands`.
- `src/cli.rs` — clap `Cli` / `Command` definitions.
- `src/commands/` — one thin module per domain (`account`, `status`, `history`,
  `pay`, `bill`, `search`, `district`, `config`, `auth`, `accounts`,
  `self_update`, `completions`); `mod.rs` holds `Ctx` and the shared
  `resolve_account`.
- `src/client.rs` — the `Wipp` HTTP client: tenant header, browser UA, the async
  submit-then-poll `/requests/{id}` pattern, and the FIS login (two hops on a
  cookie session → `id_token`).
- `src/acct.rs` — account-id parsing (`NNNNNNN-N` ↔ padded API key). Tested.
- `src/account.rs` — normalized `Account`/`ServiceCharge` + amount-due math. Tested.
- `src/bill.rs` — parses the hosted PDF bill's text (`[KEY=VALUE]` block + labeled
  lines) into a `Bill` (owner, mailing address, AutoPay). Tested. Uses
  `pdf-extract` to read the PDF; the bill carries owner data the JSON API redacts.
- `src/model.rs` — `District`, `Payment`, `AccountStatus`, `AccountMatch` models. Tested.
- `src/auth/` — `secrets.rs` (`Secret` redacts/zeroizes + keychain store; tested)
  and `session.rs` (login/logout; stores the password, mints tokens per call).
- `src/formatter.rs` — human vs `--json` rendering (shows what the API returns).
- `src/util.rs` — date math, prompts, browser opener, JWT-claims decode. Tested.
- `src/version.rs` — `VERSION` + `build_info`. Tested.
- `src/config.rs` — saved default-account + login-email files (not secrets).
- `src/update.rs` — `self-update` via GitHub releases (`self_update` crate).
- `docs/wipp-api.md` — the discovered API (endpoints, shapes, gotchas). No PII.
- `.github/workflows/release.yml` — tag `v*` → build binaries `self-update` fetches.

## When chaining as an agent

- Use `--json`. stdout is pure JSON; stderr is diagnostics.
- Respect exit codes (see README): `2` usage, `3` auth, `4` not found, `5` network,
  `6` rate-limited, `7` async timeout.
- The field you usually want is `.balance.amount` from `summary`/`balance`
  (utility-summary/v1), or `.items[].amount.amount` from `history`
  (payment-list/v1). Amounts are string-decimal dollars, dates ISO.

## Gotchas & rules

- **No secrets, no PII in the repo.** Not in code, tests, fixtures, or commits.
  Tests use synthetic account data (`1234567-8`). CI-style secret scanning runs
  via `.githooks/pre-commit` (gitleaks). The one real account number that appears
  is only ever typed at runtime by the user — never bake it in.
- **Mirror the provider — don't impose privacy it doesn't.** The CLI shows
  whatever the portal makes available for an account, owner name/address included.
  The portal serves anonymous account-number lookups and address searches, no
  login. The JSON API's `RedactOwnerName` flag blanks the owner name, but treat it
  as cosmetic, **not** a privacy boundary: the same account's official PDF bill —
  fetched over the same anonymous channel — still carries owner + mailing address,
  so `bill` and `search --full` surface them. The tool renders what the provider
  exposes; there is no owner-redaction flag, and sanitizing shared output is the
  user's call. (The separate "no PII **in the repo**" rule is unaffected —
  tests/fixtures stay synthetic; that rule is about committed code, never runtime
  output.)
- The API's WAF blocks non-browser User-Agents — `client.rs` must send a
  browser-shaped UA. If reads start 403'ing, that's the first thing to check.
- Responses **drift** and some errors arrive as a bare `(NNN) message` string;
  parsing is best-effort. If a field goes missing, fix the path and add a test —
  don't make it required.
- Be polite: this is a public portal at human scale. No aggressive looping.
- Card payments intentionally go through the portal (reCAPTCHA + gateway). Don't
  add code that posts card data.
- **Auth is the SunGard/FIS flow, verified against a live login.** Login is two
  hops on one cookie session: `POST /proxy/fis/rest/1.0/sessions {loginName,
  password}` (api-type: auth) → session cookies, then `GET
  /proxy/fis/rest/1.0/idptoken/openid-connect?client_id=Enroll.User` → an
  `id_token` JWT used as the bearer. Wrong passwords come back `400` on hop 1. The
  client needs `cookie_store(true)`. The password is the persisted credential
  (keychain) — FIS exposes no refresh token. Cognito-only endpoints
  (`/accounts/cognitoUsers`, `/payments/schedules`, `/wallet/Accounts`) 403/500
  for FIS users and are intentionally not shipped; `/accounts/billingAccounts`
  works and powers `accounts`.

## The CLI family & cli-common

This CLI conforms to **piekstra-cli/1** — the shared surface spec in
[piekstra/cli-common](https://github.com/piekstra/cli-common) (`DESIGN.md`):
standard `auth` / `config` / `self-update` / `completions` / `info` commands,
global `--json`, canonical DTOs (`auth-status/v1`, `self-update/v1`,
`cli-info/v1`), and frozen exit codes 0–6.

- **Don't fork shared behavior.** Error/exit-code handling, output rendering,
  keychain secrets, config storage, and self-update come from the `pk-cli-*`
  crates (tag-pinned git deps on cli-common). If you need a change there — or
  you're writing anything reusable across the family CLIs (fpl, xfin, lrfl,
  tojfl, …) — add it to cli-common, cut a tag, and bump the pin here. Never
  copy shared code into this repo.
- **Surface changes are spec changes.** A new standard command, flag, DTO
  field, or exit code belongs in cli-common's `DESIGN.md` first; update
  `conformance.md` alongside.
- **macOS dev signing.** Every plain `cargo build` gets a fresh ad-hoc code
  signature, so keychain "Always Allow" grants don't stick and every rebuild
  re-prompts. One-time: run cli-common's `scripts/setup-dev-signing.sh`. Then
  build with `make dev` (build + re-sign with the stable `pk-cli-codesign`
  identity) whenever you'll exercise keychain-touching commands.
