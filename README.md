# loxahatchee-river-fl

Check your **Loxahatchee River District** utility account — balance, charges,
service status, and payment history — from the command line, and jump straight
to the district's secure page to pay.

It's a thin, polite client over the same **WIPP** (Edmunds GovTech / SunGard)
API that the district's payment portal at
[`wipp.edmundsgovtech.cloud`](https://wipp.edmundsgovtech.cloud/home?wippId=LOXA)
uses. Every read is an anonymous **guest-view** lookup by account number — the
same call the portal makes before you log in — so there's **no account,
password, or API key** to configure.

Built to be **human- and agent-friendly**: every command has a `--json` mode and
stable exit codes, so a person or a script can chain it into a pipeline.

> The crate/repo is `loxahatchee-river-fl`; the binary is **`lrfl`**.

## Install

```sh
cargo build --release        # binary at ./target/release/lrfl
# or
cargo install --path .       # installs the `lrfl` binary onto your PATH
```

Requires a recent stable Rust toolchain.

## Quick start

```sh
# Remember your account so later commands need no argument:
lrfl config set-account 1234567-0

lrfl balance                 # what do I owe?
lrfl charges                 # per-service detail, meter readings, usage
lrfl history --years 2       # recent payments
lrfl status                  # which services are active
lrfl pay --open              # compute the amount and open the Pay Now page

lrfl search "MAPLE"          # find accounts by address (no account number needed)
lrfl search "MAPLE" --full   # …with owner, mailing addr, AutoPay & balance folded in
lrfl bill 1234567-0          # the current bill (owner, mailing addr, AutoPay) from the PDF
```

`search` gives you an account number; feed it to `bill`, `account`, or `balance`
for full detail — or pass `--full` to `search` to fold that detail into each
match in one shot (it fetches a bill per match, so it caps to a small result set).

You can always pass an account explicitly (`lrfl balance 1234567-0`), or set
`$LRFL_ACCOUNT` instead of saving a default.

## Commands

| Command | What it shows |
|---------|---------------|
| `lrfl summary [ACCT]` | One-shot overview: balance, per-service status, last payment |
| `lrfl balance [ACCT]` | Amount due, per service and total |
| `lrfl account [ACCT]` | Full record: balance, service location, owner name/address |
| `lrfl charges [ACCT]` | Principal/interest, due dates, billed-YTD, meter readings & usage |
| `lrfl status [ACCT]` | Each service's active/inactive status |
| `lrfl history [ACCT]` | Posted payments (`--since YYYY-MM-DD` or `--years N`) |
| `lrfl pay [ACCT]` | Compute the amount due and hand off to the portal's Pay Now page (`--open`) |
| `lrfl open [ACCT]` | Open the account's portal page in your browser |
| `lrfl bill [ACCT]` | The current bill from the official PDF: bill-to owner, mailing address, AutoPay, period, total due (`--open` opens the PDF, `--save PATH` downloads it) |
| `lrfl search <ADDR>` | Find accounts by street/property address (`--limit N`; `-b/--balances` adds each match's balance; `--full` folds in each match's bill detail) — no login |
| `lrfl district` | District info: billed services, payment options, contact |
| `lrfl config …` | `set-account`, `show`, `clear` the saved default account |
| `lrfl login` / `logout` / `whoami` | Manage a logged-in session (credential in the OS keychain) |
| `lrfl accounts [-b/--balances]` | Utility accounts linked to your login (with amounts due) — requires login |
| `lrfl self-update` | Update `lrfl` to the latest GitHub release (`--check` to only check) |
| `lrfl completions <shell>` | Print a shell completion script |

### Logging in

Guest reads (`balance`, `charges`, `history`, …) need no login. Logging in adds
`whoami` and `accounts` (the utility accounts linked to your portal login):

```sh
lrfl login                    # prompts for email + password (no-echo)
lrfl whoami                   # who you're logged in as (name, user id)
lrfl accounts --balances      # linked accounts, with what you owe on each
lrfl logout                   # removes the stored session
```

Once you're logged in, the account-scoped commands need **no account argument** —
`lrfl balance`, `charges`, `history`, `status`, `pay` fall back to the utility
account linked to your login (as long as you haven't set a different default with
`config set-account`). If your login has more than one linked account, they'll ask
you to pick one.

Loxahatchee logins go through the district's SunGard/FIS identity provider, whose
session is cookie-based with no long-lived token to persist. So `lrfl` stores your
**password in the OS keychain** (macOS Keychain) and performs a fresh login to
mint a short-lived token at the start of each authenticated command — nothing
expirable is kept, and the password is never written to disk in plaintext.
`--email` / `$LRFL_EMAIL` pick the account; `$LRFL_PASSWORD` can supply the
password in headless/CI use. You can pipe the password on stdin
(`printf '%s' "$PW" | lrfl login --email you@example.com`) for scripting.

> On macOS the first authenticated command after installing (or upgrading) the
> binary shows a Keychain permission prompt — click **Always Allow** so future
> runs don't ask again. This is macOS tying keychain access to each build.

### Staying up to date

```sh
lrfl self-update --check      # is a newer release available?
lrfl self-update              # download + replace the binary in place
```

### Shell completions

```sh
lrfl completions zsh  > ~/.zfunc/_lrfl      # then ensure ~/.zfunc is in $fpath
lrfl completions bash > /usr/local/etc/bash_completion.d/lrfl
```

### Account numbers

A utility account is `NNNNNNN-N` — the base number and its check digit, exactly
as shown on your bill and in the portal URL (e.g. `1234567-0`).

### Paying

The portal exposes no way to submit a card programmatically: the payment step is
guarded by reCAPTCHA and card entry happens on the payment processor's own hosted
page (BluePay/FIS). So `lrfl pay` automates everything up to that wall — it
authenticates, computes exactly what you owe, and hands you the district's secure
Pay Now page for the account (`--open` launches it); you enter the card and submit
there. The card never passes through `lrfl`, because there's no supported path for
it to — reCAPTCHA and a processor-hosted form are exactly the wall that stops it.

## JSON & scripting

```sh
# Just the number you owe (utility-summary/v1 — string-decimal dollars):
lrfl balance --json | jq -r '.balance.amount'

# Total paid in the last 3 years (payment-list/v1 — records under .items):
lrfl history --json | jq '[.items[].amount.amount | tonumber] | add'

# Machine-readable service status:
lrfl status --json | jq '{sewer, overall}'
```

In `--json` mode the JSON document is the only thing on **stdout**; diagnostics
go to **stderr**, so `| jq` is always safe.

## Privacy

The CLI shows whatever the district's portal returns for an account — owner name
and mailing address included. It doesn't impose privacy the provider itself
doesn't: the portal answers anonymous account-number lookups and address
searches, no login required. The district's JSON API happens to blank the owner
name (its `RedactOwnerName` flag), but that's cosmetic, not a privacy boundary —
the same account's official PDF bill, fetched over the same anonymous channel,
still carries the owner and mailing address, so `lrfl bill` (and `search --full`)
surface them. The tool faithfully renders what the provider makes available; it
doesn't invent redaction the provider doesn't enforce. If you paste output
somewhere public, sanitizing it is your call.

The only secret the tool stores is your portal **password**, in the OS keychain
(the FIS session model exposes no long-lived token to keep instead) — never in a
plaintext file, and redacted/zeroized in memory. Non-secret state — your default
account number and login email — lives in plain files under
`~/.config/loxahatchee-cli/`. Guest reads store nothing at all.

## Global flags

| Flag | Description |
|------|-------------|
| `--json` | Machine-readable JSON on stdout |
| `-v, --verbose` | Extra diagnostics on stderr |
| `-q, --quiet` | Suppress non-error stderr |
| `--no-color` | Disable ANSI color (reserved) |
| `--wipp-id <ID>` | WIPP tenant id (default `LOXA`; or `$LRFL_WIPP_ID`) |
| `--email <EMAIL>` | Login email for authenticated commands (or `$LRFL_EMAIL`) |
| `-V, --version` | Print version |
| `-h, --help` | Help |

## Exit codes

| Code | Meaning |
|------|---------|
| `0` | success |
| `1` | generic error (incl. keychain failure) |
| `2` | usage error (bad args / no account) |
| `3` | authentication required or failed |
| `4` | not found (no such account) |
| `5` | network / upstream error |
| `6` | rate-limited (HTTP 429) |
| `7` | timed out waiting on an async server request |

## How it works

The portal is a SunGard/Edmunds "WIPP" tenant (`LOXA`). Its API lives at
`api.edmundsgovtech.cloud/wipp-core/v1`, selects a tenant with the `X-Wipp-Id`
header, and (via an AWS WAF) expects a browser User-Agent. Account lookups are
plain guest reads; a couple of endpoints (like service status) are asynchronous —
you submit and poll `GET /requests/{id}` until it's ready. See
[`docs/wipp-api.md`](docs/wipp-api.md) for the discovered endpoints.

## Scope & disclaimer

A personal-use tool for viewing your own publicly available utility-account data
at human scale. Not affiliated with or endorsed by the Loxahatchee River
District, Edmunds GovTech, or SunGard. Endpoints are undocumented and may change.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your
option. See [`CONTRIBUTING.md`](CONTRIBUTING.md).
