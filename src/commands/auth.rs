//! `login` / `logout` / `whoami` — manage the keychain-backed login session.

use pk_cli_auth::{AuthMethod, AuthStatus};

use crate::cli::AuthCmd;
use crate::commands::Ctx;
use crate::error::AppError;
use crate::formatter;
use crate::util;

pub fn run(ctx: &Ctx, cmd: &AuthCmd) -> Result<(), AppError> {
    match cmd {
        AuthCmd::Login => login(ctx),
        AuthCmd::Logout => logout(ctx),
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
            "summary", "account", "balance", "charges", "status", "history", "pay", "district",
            "accounts",
        ],
    )
    .with_profiles(&[pk_cli_utility::PROFILE]);
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

pub fn logout(ctx: &Ctx) -> Result<(), AppError> {
    let login = ctx.session.resolve_login(ctx.login.as_deref())?;
    let removed = ctx.session.logout(&login)?;
    if ctx.json {
        println!(
            "{}",
            serde_json::json!({ "login": login, "removed": removed })
        );
    } else if !ctx.quiet {
        if removed {
            println!("✓ logged out {login}");
        } else {
            println!("no stored session for {login}");
        }
    }
    Ok(())
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
