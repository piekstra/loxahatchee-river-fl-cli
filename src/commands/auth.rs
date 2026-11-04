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
    // A connector token can outlive a login, so `--forget` works with no login to clear.
    let login = match ctx.session.resolve_login(ctx.login.as_deref()) {
        Ok(login) => Some(login),
        Err(_) if forget => None,
        Err(e) => return Err(e),
    };
    let removed = match &login {
        Some(login) => ctx.session.logout(login)?,
        None => false,
    };
    let token =
        forget.then(|| CredentialStore::for_binary("lrfl").delete(pk_cli_mcp::HTTP_TOKEN_ACCOUNT));
    if forget {
        config::clear_default_account();
    }
    // Before anything is printed, so a failure is the one JSON document of this call.
    let token_removed = token.map(finish_forget).transpose()?;
    if ctx.json {
        let mut out = serde_json::json!({ "login": login, "removed": removed });
        if let Some(token_removed) = token_removed {
            out["connector_token_removed"] = token_removed.into();
        }
        println!("{out}");
    } else if !ctx.quiet {
        match (&login, removed) {
            (Some(login), true) => println!("✓ logged out {login}"),
            (Some(login), false) => println!("no stored session for {login}"),
            (None, _) => println!("no login to clear"),
        }
    }
    if token_removed == Some(true) && !ctx.quiet {
        eprintln!("{}", pk_cli_mcp::stop_note("lrfl"));
    }
    Ok(())
}

/// Whether `auth logout --forget` removed a connector token. Nothing records whether `mcp
/// --new-token` ran, so the delete is ungated, and one that could not be removed is an error,
/// since it still opens the connector.
fn finish_forget(token: Result<bool, AppError>) -> Result<bool, AppError> {
    token.map_err(|failure| {
        AppError::Keychain(format!(
            "the connector token of `lrfl mcp --http` could not be removed, if there is one, and \
             would still open the connector: {}. Run `lrfl auth logout --forget` again.",
            failure.message()
        ))
    })
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
    use crate::error::AppError;

    #[test]
    fn a_forget_that_could_not_remove_the_connector_token_says_so() {
        let denied = AppError::Keychain("denied".into());
        let says = super::finish_forget(Err(denied)).unwrap_err().to_string();
        assert!(says.contains("connector token"), "{says}");
        assert!(says.contains("`lrfl auth logout --forget`"), "{says}");
        assert!(says.contains("denied"), "{says}");
    }

    #[test]
    fn a_forget_with_a_token_or_without_one_finishes() {
        assert!(super::finish_forget(Ok(true)).unwrap());
        assert!(!super::finish_forget(Ok(false)).unwrap());
    }
}
