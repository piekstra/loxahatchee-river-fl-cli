//! Login session management.
//!
//! Loxahatchee authenticates through the SunGard/FIS identity provider, whose
//! session is **cookie-based** — there is no long-lived refresh token exposed to
//! the client, only a short-lived `id_token` derived from a cookie session (see
//! [`crate::client::Wipp::fis_login`]). To give the CLI a persistent login
//! without keeping an expirable cookie around, we store the **password** in the
//! OS keychain and perform a fresh two-hop login at the start of each
//! authenticated command, yielding a fresh `id_token` each time.
//!
//! Keychain layout: service `com.piekstra.loxahatchee-river-fl`, account = the
//! login name, secret = the password. The login name (not a secret) is
//! remembered in `~/.config/loxahatchee-cli/session-email`.

use crate::client::Wipp;
use crate::config;
use crate::error::AppError;

use super::secrets::{CredentialStore, Secret, SourceKind};

/// Keychain service name (reverse-DNS).
pub const SERVICE: &str = "com.piekstra.loxahatchee-river-fl";
/// Environment fallback for the password (e.g. for CI/headless use).
pub const ENV_PASSWORD: &str = "LRFL_PASSWORD";
/// Environment fallback for the login name.
pub const ENV_LOGIN: &str = "LRFL_EMAIL";

/// Owns the keychain-backed session and drives login / logout / token minting.
pub struct Session {
    store: CredentialStore,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    pub fn new() -> Self {
        Session {
            store: CredentialStore::new(SERVICE),
        }
    }

    /// Resolve the login name: explicit value, then `$LRFL_EMAIL`, then the name
    /// saved by a previous `lrfl login`.
    pub fn resolve_login(&self, login: Option<&str>) -> Result<String, AppError> {
        if let Some(e) = login {
            if !e.trim().is_empty() {
                return Ok(e.trim().to_string());
            }
        }
        if let Ok(e) = std::env::var(ENV_LOGIN) {
            if !e.is_empty() {
                return Ok(e);
            }
        }
        config::load_session_email().ok_or_else(|| {
            AppError::Auth(
                "not logged in — run `lrfl login` (or pass --email / set $LRFL_EMAIL)".into(),
            )
        })
    }

    /// Verify credentials via a full login, then store the password so future
    /// commands can mint fresh tokens.
    pub fn login(&self, api: &Wipp, login: &str, password: &Secret) -> Result<(), AppError> {
        // Prove the credentials work (and surface a clear error if not) before
        // persisting anything.
        api.fis_login(login, password.expose())?;
        self.store.set(login, password)?;
        let _ = config::save_session_email(login);
        Ok(())
    }

    /// Whether `$LRFL_PASSWORD` is set to something other than `stored`, so that
    /// commands will keep using the variable and not what was just stored.
    pub fn env_overrides(stored: &Secret) -> bool {
        env_differs(std::env::var(ENV_PASSWORD).ok().as_deref(), stored)
    }

    /// Remove the stored session for `login`. Returns whether anything was
    /// removed from the keychain.
    pub fn logout(&self, login: &str) -> Result<bool, AppError> {
        let removed = self.store.delete(login)?;
        if config::load_session_email().as_deref() == Some(login) {
            config::clear_session_email();
        }
        Ok(removed)
    }

    /// Is a stored credential present for `login` (keychain or env)?
    pub fn has_credential(&self, login: &str) -> bool {
        self.store
            .get(login)
            .ok()
            .flatten()
            .map(|s| !s.is_empty())
            .unwrap_or(false)
            || std::env::var(ENV_PASSWORD)
                .map(|v| !v.is_empty())
                .unwrap_or(false)
    }

    /// Mint a fresh `id_token` for `login` by performing the FIS login with the
    /// stored password. A refusal of a password from `$LRFL_PASSWORD` says so:
    /// `lrfl auth login` would store one the variable keeps overriding.
    pub fn access_token(&self, api: &Wipp, login: &str) -> Result<String, AppError> {
        let password = crate::auth::secrets::resolve(&self.store, login, ENV_PASSWORD)?;
        api.fis_login(login, password.secret.expose())
            .map_err(|e| match (e, password.source) {
                (AppError::Auth(message), SourceKind::Env) => AppError::Auth(format!(
                    "{message} (the password came from ${ENV_PASSWORD}, which comes before the \
                     keychain)"
                )),
                (e, _) => e,
            })
    }
}

fn env_differs(env: Option<&str>, stored: &Secret) -> bool {
    env.is_some_and(|value| value != stored.expose())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_different_variable_overrides_what_was_stored() {
        let stored = Secret::new("stored");
        assert!(env_differs(Some("old"), &stored));
        assert!(env_differs(Some(""), &stored));
        assert!(!env_differs(Some("stored"), &stored));
        assert!(!env_differs(None, &stored));
    }
}
