//! A tiny bit of on-disk state: the default account number, so day-to-day
//! commands can be run as `lrfl balance` with no argument.
//!
//! An account number is **not** a credential — this tool stores no secrets — so
//! it lives in a plain file under `$XDG_CONFIG_HOME` (or `~/.config`). Nothing
//! here ever touches the keychain.

use std::path::PathBuf;

/// Base config directory (`$XDG_CONFIG_HOME` or `~/.config`) for this tool.
fn config_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("loxahatchee-cli"))
}

/// Location of the remembered default-account file.
fn default_account_path() -> Option<PathBuf> {
    Some(config_dir()?.join("account"))
}

/// Location of the remembered login-email file (an email is not a secret; the
/// password it pairs with lives in the OS keychain).
fn session_email_path() -> Option<PathBuf> {
    Some(config_dir()?.join("session-email"))
}

/// The remembered default account, if one was saved.
pub fn load_default_account() -> Option<String> {
    let s = std::fs::read_to_string(default_account_path()?).ok()?;
    let s = s.trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// Remember `account` as the default. Best-effort; failures are non-fatal.
pub fn save_default_account(account: &str) -> std::io::Result<()> {
    let path =
        default_account_path().ok_or_else(|| std::io::Error::other("no config directory"))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, account)
}

/// Forget the default account. Returns `true` if a file was removed.
pub fn clear_default_account() -> std::io::Result<bool> {
    match default_account_path() {
        Some(p) => match std::fs::remove_file(p) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e),
        },
        None => Ok(false),
    }
}

/// Human-readable path to the config file, for `config` output.
pub fn config_path_display() -> String {
    default_account_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "(unavailable)".into())
}

/// The remembered login email, if `lrfl login` saved one.
pub fn load_session_email() -> Option<String> {
    let s = std::fs::read_to_string(session_email_path()?).ok()?;
    let s = s.trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// Remember `email` as the logged-in account. Best-effort.
pub fn save_session_email(email: &str) -> std::io::Result<()> {
    let path = session_email_path().ok_or_else(|| std::io::Error::other("no config directory"))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, email)
}

/// Forget the saved login email. Returns `true` if a file was removed.
pub fn clear_session_email() -> bool {
    match session_email_path() {
        Some(p) if p.exists() => std::fs::remove_file(p).is_ok(),
        _ => false,
    }
}
