# Contributing to loxahatchee-river-fl

Thanks for helping out! This is a small, focused CLI over the Loxahatchee River
District's WIPP utility-billing portal.

## Before you start

- Open or comment on an issue describing the change.
- Branch from `main` (`feat/…`, `fix/…`, `docs/…`); don't push to `main` directly.
- Use [Conventional Commits](https://www.conventionalcommits.org/).

## Local checks (must pass)

```sh
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

If you touch API parsing, add/adjust a unit test with a **synthetic** JSON node
(see `src/account.rs`). Keep fields best-effort — the mainframe responses drift
and a missing field shouldn't crash a lookup.

## Ground rules

- **No secrets or PII** in code, tests, fixtures, or commits — real account
  numbers, names, addresses, or balances included. This tool needs no
  credentials; keep it that way. CI runs `gitleaks` on every pull request.
- The CLI shows whatever the portal returns (owner name/address included); it
  doesn't add redaction the provider itself doesn't. (The repo-hygiene rule above
  is separate — it's about committed code, never runtime output.)
- Card payments go through the district's official gateway — don't add code that
  captures or posts card data.
- Keep it personal-scale and polite to the portal. No bulk scraping or hammering.
- Be kind — see [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md).

## License of contributions

By contributing, you agree your work is licensed under **AGPL-3.0-only**.

## The CLI family & cli-common

This repo is part of a family of CLIs (fpl, xfin, lrfl, tojfl, …) that share a
surface spec and library crates: [piekstra/cli-common](https://github.com/piekstra/cli-common)
(**piekstra-cli/1**). Before adding anything reusable — output rendering,
secret handling, config storage, self-update, DTO shapes — check whether it
belongs in cli-common's `pk-cli-*` crates instead. Contributions of shared,
reusable pieces to cli-common are encouraged and preferred over per-repo
copies; consume them here as tag-pinned git dependencies.

Surface changes (new standard commands/flags, DTO fields, exit codes) start as
a spec change in cli-common's `DESIGN.md`.

On macOS, run cli-common's `scripts/setup-dev-signing.sh` once and build with
`make dev` so keychain "Always Allow" grants survive rebuilds.
