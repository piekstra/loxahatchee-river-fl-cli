//! `login` / `logout` / `whoami` — manage the keychain-backed login session.

use pk_cli_auth::{AuthMethod, AuthStatus};
use pk_cli_secrets::CredentialStore;

use crate::auth::session::ENV_PASSWORD;
use crate::auth::Session;
use crate::cli::AuthCmd;
use crate::commands::Ctx;
use crate::config;
use crate::error::AppError;
use crate::formatter;
use crate::util;

pub fn run(ctx: &Ctx, cmd: &AuthCmd) -> Result<(), AppError> {
    match cmd {
        AuthCmd::Login => login(ctx),
        AuthCmd::Logout(args) => logout(ctx, args.forget),
        AuthCmd::Status => status(ctx),
        AuthCmd::Whoami => whoami(ctx),
    }
}

/// `auth status` — canonical auth-status/v1. Reads are anonymous guest-view
/// lookups, so no credential is *required*; one enables `accounts`/`whoami`.
pub fn status(ctx: &Ctx) -> Result<(), AppError> {
    let login = ctx.session.resolve_login(ctx.login.as_deref()).ok();
    let has_credential = login
        .as_deref()
        .map(|l| ctx.session.has_credential(l))
        .unwrap_or(false);
    let mut st = AuthStatus::new(false, true, AuthMethod::Password);
    st.username = login;
    st.credential_in_keychain = Some(has_credential);
    st.emit(ctx.json);
    Ok(())
}

/// `info` — cli-info/v1 capability discovery.
pub fn info(ctx: &Ctx) -> Result<(), AppError> {
    use pk_cli_core::info::{AuthInfo, CliInfo};
    let _ = ctx;
    let info = CliInfo::new(
        "lrfl",
        env!("CARGO_PKG_VERSION"),
        "https://github.com/piekstra/loxahatchee-river-fl-cli",
        AuthInfo {
            required: false,
            method: "password".into(),
            login_hint: Some("lrfl auth login".into()),
        },
        &[
            "summary",
            "account",
            "balance",
            "charges",
            "status",
            "history",
            "pay",
            "district",
            "accounts",
            "documents",
            "mcp",
        ],
    )
    .with_profiles(&[pk_cli_utility::PROFILE, pk_cli_documents::PROFILE]);
    pk_cli_core::output::json(&serde_json::to_value(&info).unwrap_or_default());
    Ok(())
}

pub fn login(ctx: &Ctx) -> Result<(), AppError> {
    let login = match ctx.session.resolve_login(ctx.login.as_deref()) {
        Ok(l) => l,
        Err(_) => util::prompt_line("Portal email")?,
    };
    let password = util::read_password(&format!("Password for {login}: "))?;
    if password.is_empty() {
        return Err(AppError::Usage(
            "empty password — nothing to log in with".into(),
        ));
    }
    ctx.log(&format!("authenticating {login}…"));
    ctx.session.login(&ctx.api, &login, &password)?;
    if Session::env_overrides(&password) {
        eprintln!(
            "warning: ${ENV_PASSWORD} is set to a different password and comes before the \
             keychain, so commands will keep using it until you change or unset it"
        );
    }
    if ctx.json {
        println!(
            "{}",
            serde_json::json!({ "login": login, "logged_in": true, "store": "keychain" })
        );
    } else if !ctx.quiet {
        println!("✓ logged in as {login} — password stored in the OS keychain");
    }
    Ok(())
}

pub fn logout(ctx: &Ctx, forget: bool) -> Result<(), AppError> {
    let login = login_to_clear(ctx.session.resolve_login(ctx.login.as_deref()), forget)?;
    let session = match &login {
        Some(login) => ctx.session.logout(login),
        None => Ok(false),
    };
    let (removed, token_removed) = if forget {
        let token = CredentialStore::for_binary("lrfl").delete(pk_cli_mcp::HTTP_TOKEN_ACCOUNT);
        let account = config::clear_default_account()
            .map(drop)
            .map_err(|e| AppError::Other(format!("removing the default account: {e}")));
        // Before anything is printed, so a failure is the one JSON document of this call.
        let (removed, token_removed) = finish_forget(session, token, account, ctx.quiet)?;
        (removed, Some(token_removed))
    } else {
        (session?, None)
    };
    if ctx.json {
        println!("{}", logout_json(login.as_deref(), removed, token_removed));
    } else if !ctx.quiet {
        match (&login, removed) {
            (Some(login), true) => println!("✓ logged out {login}"),
            (Some(login), false) => println!("no stored session for {login}"),
            (None, _) => println!("no login to clear"),
        }
    }
    Ok(())
}

/// The login a logout clears. A connector token can outlive a login, so `--forget` works with no
/// login known. Any other failure to find one is still a failure.
fn login_to_clear(
    resolved: Result<String, AppError>,
    forget: bool,
) -> Result<Option<String>, AppError> {
    match resolved {
        Ok(login) => Ok(Some(login)),
        Err(e) if forget && e.is_auth() => Ok(None),
        Err(e) => Err(e),
    }
}

/// `auth logout --json`. `login` and `connector_token_removed` are left out when they have no
/// value: no login was known, or this was not a `--forget`.
fn logout_json(
    login: Option<&str>,
    removed: bool,
    token_removed: Option<bool>,
) -> serde_json::Value {
    let mut out = serde_json::json!({ "removed": removed });
    if let Some(login) = login {
        out["login"] = login.into();
    }
    if let Some(token_removed) = token_removed {
        out["connector_token_removed"] = token_removed.into();
    }
    out
}

/// Whether `auth logout --forget` removed a session and a connector token. Nothing records whether
/// `mcp --new-token` ran, so the token's delete is ungated. Every step is attempted and every failure
/// reported here: none hides another, or the note that a removed token still needs, since a rerun
/// finds no item.
fn finish_forget(
    session: Result<bool, AppError>,
    token: Result<bool, AppError>,
    account: Result<(), AppError>,
    quiet: bool,
) -> Result<(bool, bool), AppError> {
    if matches!(token, Ok(true)) && !quiet {
        eprintln!("{}", pk_cli_mcp::stop_note("lrfl"));
    }
    match (session, token, account) {
        (Ok(removed), Ok(token_removed), Ok(())) => Ok((removed, token_removed)),
        (Ok(_), Ok(_), Err(e)) => Err(e),
        (session, token, account) => {
            let mut failed = Vec::new();
            if let Err(e) = session {
                failed.push(format!(
                    "the stored session could not be removed ({})",
                    e.message()
                ));
            }
            if let Err(e) = token {
                failed.push(format!(
                    "the connector token of `lrfl mcp --http` could not be removed ({}) and would \
                     still open the connector",
                    e.message()
                ));
            }
            if let Err(e) = account {
                failed.push(e.message().to_string());
            }
            Err(AppError::Keychain(format!(
                "{}; run `lrfl auth logout --forget` again",
                failed.join(", and ")
            )))
        }
    }
}

pub fn whoami(ctx: &Ctx) -> Result<(), AppError> {
    let login = ctx.session.resolve_login(ctx.login.as_deref()).ok();
    match &login {
        Some(l) if ctx.session.has_credential(l) => {
            ctx.log("verifying the stored session with the portal…");
            // Minting a token both confirms the credential still works and yields
            // the identity claims to display.
            let token = ctx.session.access_token(&ctx.api, l)?;
            let claims = util::decode_jwt_claims(&token);
            formatter::print_whoami(Some(l), claims.as_ref(), ctx.json);
        }
        other => formatter::print_whoami(other.as_deref(), None, ctx.json),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{finish_forget, login_to_clear, logout_json};
    use crate::error::AppError;
    use serde_json::json;

    #[test]
    fn only_a_forget_goes_on_with_no_login_known() {
        let none = || Err(AppError::Auth("not logged in".into()));
        assert_eq!(login_to_clear(none(), true).unwrap(), None);
        assert_eq!(login_to_clear(none(), false).unwrap_err().exit_code(), 3);
        let known = login_to_clear(Ok("dummy@example.com".into()), false).unwrap();
        assert_eq!(known.as_deref(), Some("dummy@example.com"));
    }

    #[test]
    fn any_other_failure_to_find_a_login_stays_a_failure() {
        let broken = Err(AppError::Keychain("denied".into()));
        assert!(login_to_clear(broken, true).is_err());
    }

    #[test]
    fn the_json_answer_holds_only_what_has_a_value() {
        let plain = logout_json(Some("dummy@example.com"), true, None);
        assert_eq!(
            plain,
            json!({"login": "dummy@example.com", "removed": true})
        );
        let forgot = logout_json(None, false, Some(true));
        assert_eq!(
            forgot,
            json!({"removed": false, "connector_token_removed": true})
        );
    }

    #[test]
    fn a_forget_that_could_not_remove_the_connector_token_says_so() {
        let denied = AppError::Keychain("denied".into());
        let says = finish_forget(Ok(true), Err(denied), Ok(()), true)
            .unwrap_err()
            .to_string();
        assert!(says.contains("connector token"), "{says}");
        assert!(says.contains("`lrfl auth logout --forget`"), "{says}");
        assert!(says.contains("denied"), "{says}");
    }

    #[test]
    fn no_failure_of_a_forget_hides_another() {
        let says = finish_forget(
            Err(AppError::Keychain("locked".into())),
            Err(AppError::Keychain("denied".into())),
            Err(AppError::Other("read-only".into())),
            true,
        )
        .unwrap_err()
        .to_string();
        for cause in ["locked", "denied", "read-only"] {
            assert!(says.contains(cause), "{says}");
        }
        let e = finish_forget(
            Ok(true),
            Ok(true),
            Err(AppError::Other("read-only".into())),
            true,
        )
        .unwrap_err();
        assert!(e.to_string().contains("read-only"), "{e}");
    }

    #[test]
    fn a_forget_says_what_it_removed() {
        assert_eq!(
            finish_forget(Ok(true), Ok(true), Ok(()), true).unwrap(),
            (true, true)
        );
        assert_eq!(
            finish_forget(Ok(false), Ok(false), Ok(()), true).unwrap(),
            (false, false)
        );
    }
}
