# Security policy

## Reporting a vulnerability

Please report security issues **privately** — do not open a public issue.

- Preferred: open a [GitHub private security advisory](https://docs.github.com/en/code-security/security-advisories)
  on this repo ("Security" tab → "Report a vulnerability").
- Or contact the maintainer.

We aim to acknowledge within a few days and coordinate a fix/disclosure with you.

## Notes for this tool

Guest reads are anonymous and store nothing. If you `lrfl login`, the tool holds
one secret: your portal **password**, stored in the **OS keychain** (macOS
Keychain / Windows Credential Manager / Linux Secret Service). The district's
SunGard/FIS login is cookie-based and hands the client no long-lived token to
keep instead, so the password is the durable credential; it is never written to
disk in plaintext. A fresh, short-lived session token is minted per command and
kept only in memory. Non-secret state (default account number, login email) lives
in plain `~/.config` files.

The main concerns are therefore:

- **Credential handling.** Secrets are wrapped in a type that refuses to print
  itself (`Debug`/`Display` redacted) and is zeroized on drop; nothing secret is
  logged or written to disk. `$LRFL_PASSWORD`, for CI, is read before the keychain.
- **No secrets or PII in the repo.** Owner name/address is redacted by default;
  tests and fixtures use synthetic data; the pre-commit hook runs `gitleaks`.
- **Dependency advisories** (`cargo audit` / `cargo deny`).
- **Payments stay on the official gateway.** This tool never captures card data;
  it hands off to the district's PCI-compliant Pay Now page.

## The MCP server

`lrfl mcp` serves read-only tools to an MCP client (Claude Code, Claude Desktop) over
stdio; the server opens no port unless run with `--http` (below). **What the tools
return goes to Claude on every path**, as part of your conversation: your balance, due
date, bills, payments, metered use and account number.

The tools read an account by its number through the district's anonymous lookup, so
the server signs in nowhere, reads no password and holds none, whether or not you have
run `lrfl auth login`. A call may name any account number, as `lrfl balance <ACCOUNT>`
may, because the district answers that lookup without a sign-in.

The Claude Desktop extension (`.mcpb`) takes the account number and nothing else, and
reads no keychain item.

`lrfl mcp --http --port <PORT>` serves the same tools over HTTP for the user's own
Cloudflare Worker, which reaches it through the user's own Cloudflare Tunnel
(cli-common's `connector/`). It binds 127.0.0.1 only, with no flag for another address,
and refuses to start without a bearer token of 32 or more characters, from
`LRFL_MCP_TOKEN` or the keychain item `mcp-http-token` (`piekstra.lrfl`, a different
service from your portal password's), which `lrfl mcp --new-token` makes from 256 random bits
and replaces only with `--overwrite`. **A copy of the token lives in your Cloudflare
Worker**, as its `HOME_LOXAHATCHEE_TOKEN` secret. Every request must carry the token
(compared in constant time) and a `Host` of `127.0.0.1` or `localhost`, and must not
carry an `Origin`; there are no CORS headers. Only the read-only tools are served (all
four are). That token is the only credential the server reads. The Worker sends tool
calls and receives results.

**Tool results also cross Cloudflare on this path**, through your tunnel and your
Worker, on their way to Claude. Who holds what, and what a leaked token can do, is in
the connector's
[threat model](https://github.com/piekstra/cli-common/blob/main/connector/README.md#threat-model).

`lrfl auth logout` removes the portal password and does not touch `mcp-http-token` or
stop `lrfl mcp --http`. `lrfl auth logout --forget` removes the token from the keychain
too, and says so, but a running server keeps the token it started with. To revoke the connector's access, run `lrfl mcp --new-token
--overwrite` and restart the server (`launchctl kickstart -k gui/$(id -u)/com.piekstra.lrfl-mcp-http`);
the old token works until the restart. To stop serving, run `launchctl bootout
gui/$(id -u)/com.piekstra.lrfl-mcp-http` and stop the tunnel.

## Supported versions

Pre-1.0: only the latest release receives fixes.
