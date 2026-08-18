# Changelog

## Unreleased

### Added
- **`lrfl documents list`** and **`lrfl documents download <id>`** — the same
  bill PDFs, exposed under cli-common's shared **`documents/v1`** profile so the
  file surface is spelled identically across the CLI family. A document's id is
  its ISO due date; `list` emits `document-list/v1` and `download` emits
  `document-download/v1` (or `document-download-batch/v1` with `--all -o DIR`).
  `bills get` stays as the utility-flavored alias for the same file. `lrfl info`
  now advertises the `documents/v1` profile. `documents download --all` reports
  any period with no PDF on file in the batch's `skipped` array (each with a
  machine `code` of `no_file`) rather than silently coming up short.

### Changed
- The bill-PDF download core (URL fetch, filename, directory batch with skip
  collection) moved into a shared `download` module that both `bills` and
  `documents` render over, so the two spellings of the same download can't
  drift. `bills download --all` now lists periods newest-first (was oldest) and
  its `skipped` entries are unchanged in shape.
- **`lrfl bills list`** and **`lrfl bills get <YYYY-MM-DD>`** — expose the
  historical bill surface (piekstra-cli/1's `bills list` + `bills get <id>`
  shape). `list` enumerates every discoverable period from the account payload
  (current + up to 3 prior per service, keyed by ISO due date, with a
  `statement-list/v1` envelope in `--json`); `get` downloads that period's
  official PDF from the district's hosted archive (`-o -` streams to stdout,
  `--all -o DIR` fetches every listed period into a directory). A `bills get`
  call with a due date the archive doesn't have on file exits cleanly with
  `NotFound` (exit 4) rather than saving the archive's HTML placeholder as a
  `.pdf`. Closes #13.
- `lrfl bill --save PATH` (the current-period shortcut) is unchanged and still
  works.

## v0.6.0 — 2026-07-20

### Changed
- **Adopted the `utility/v1` domain profile** (cli-common v0.2.0; `pk-cli-*`
  deps bumped v0.1.3 → v0.2.0, `pk-cli-utility` added). **Breaking for `--json`
  consumers; human/text output is unchanged:**
  - `summary --json` and `balance --json` now emit the canonical
    **`utility-summary/v1`** card: `balance` as string-decimal `Money`
    (was `balance_due` f64), the earliest per-service `due_date` (ISO), and
    `account`. Per-service detail stays in `account --json` / `charges --json`;
    status and last payment stay in `status --json` / `history --json`.
  - `history --json` now emits a **`payment-list/v1`** envelope: records under
    `items`, each a `payment/v1` (`date` ISO, `amount` as `Money`, `method`,
    `confirmation` = transaction id) — was `{ account, payments: [...] }`.
  - `info` advertises `"profiles": ["utility/v1"]`.
- **`self-update` gains `-y/--yes` and `--json`** (the `self-update/v1` DTO)
  by adopting the shared `pk-cli-selfupdate` argument surface; `--check`
  behaves as before. This restores the family-standard
  `self-update --check --json` probe that consumers (e.g. utiman) rely on — the
  previous hand-rolled `--check`-only surface didn't emit JSON.

## v0.5.0 — 2026-07-19

### Added
- **`lrfl search --full`** — fold each match's full bill detail (owner, mailing
  address, AutoPay, service period, and balance) straight into search results,
  parsed from the same official PDF `lrfl bill` uses. Because it fetches a bill
  per match, `--full` is capped to a small result set (narrow the query or lower
  `--limit`); it implies `--balances`. Human output nests the detail under each
  match; `--json` nests a `bill` object.
- `search` help now points at the `search → bill`/`account`/`balance` workflow so
  it's discoverable from `--help`, not just inferable.

### Changed
- Docs no longer lean on `RedactOwnerName` as a privacy rationale — it's cosmetic
  (blanks the owner in the JSON API/portal UI only), since the same account's
  anonymous PDF bill still carries owner + mailing address. Clarified across
  README, `docs/wipp-api.md`, and `CLAUDE.md`.

### Fixed
- Replaced non-synthetic sample data in a `bill` unit test and the `search`
  example with synthetic placeholders (repo-hygiene: keep real account/address
  data out of the source tree).

## v0.4.0 — 2026-07-19

### Added
- **`lrfl bill [ACCT]`** — the account's current bill, parsed from the official
  PDF (onlinebiller.com). Surfaces data the WIPP API redacts/omits: the bill-to
  **owner name + full mailing address** and **AutoPay status**, plus service
  address, statement date, service period, last payment, and total due. `--open`
  opens the PDF in a browser; `--save PATH` downloads it; `--json` emits the
  fields. (Owner shows for owner-billed accounts; occupant-billed ones read
  `OCCUPANT`.) Adds a pure-Rust PDF text dependency (`pdf-extract`).

## v0.3.1 — 2026-07-19

### Added
- **`lrfl search -b/--balances`** — also fetch each match's **balance due**. The
  search response has no balance, so this fans out one account lookup per match;
  use a focused query or a small `--limit`. Balances appear as a column in human
  output and as `balance_due` in `--json`. (Named to match `accounts --balances`.)

## v0.3.0 — 2026-07-19

### Added
- **`lrfl search <address>`** — find accounts by street/property address. The
  district matches server-side (case-insensitive substring on the property
  location); no login and no external geocoding are involved. `--limit N` caps
  results; `--json` emits `{ query, count, truncated, matches }`.
- **Full per-service detail** in the human `account` view: principal, interest,
  not-yet-due principal, billed-YTD, last paid, billing period, meter
  reading/usage, and early-pay discount — plus the **interest-through date**
  (`propertyInfo.interestDate`). This data was already in `--json`; now it's
  surfaced in human output too.

### Changed
- **Owner name and mailing address are shown by default.** The CLI faithfully
  renders whatever the portal returns and no longer imposes privacy the provider
  itself doesn't (the portal already serves anonymous account-number lookups and
  blanks fields on its own via `RedactOwnerName`).
- Docs corrected to state the honest reason `pay` hands off — the portal exposes
  no programmatic card API (reCAPTCHA + a processor-hosted card form) — rather
  than any PCI-compliance or repo-privacy framing.

### Removed
- **BREAKING:** the `--show-owner` global flag. Owner details are shown by
  default now, so the flag (and the redaction path behind it) is gone.

## v0.2.1 — 2026-07-11
- Homebrew formula and release polish.

## v0.2.0 — 2026-07-11
- De-duplicated the auth commands; breaking CLI cleanups toward piekstra-cli/1.

## v0.1.0 — 2026-07-11
- Initial release: anonymous guest-view account/billing/charges/history, the
  SunGard/FIS login flow, and `self-update` from GitHub releases.
