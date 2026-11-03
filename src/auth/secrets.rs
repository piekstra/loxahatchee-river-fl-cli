//! Secret handling, shared with the CLI family via `pk-cli-secrets`. The only
//! secret this tool stores is the portal **password** (the durable credential
//! in the FIS cookie-session model), set at `lrfl auth login`.

pub use pk_cli_secrets::{CredentialStore, Secret};

use pk_cli_secrets::{KeychainRead, SecretResolver, SecretSpec};

use crate::error::AppError;

/// `$env_var`, then the keychain: the family's order, so a variable set for one
/// run costs no keychain prompt. A variable set but empty is an error naming it.
/// Neither holding anything is [`AppError::Auth`].
pub fn resolve(
    store: &impl KeychainRead,
    account: &str,
    env_var: &str,
) -> Result<Secret, AppError> {
    let spec = SecretSpec::new(account)
        .env(env_var)
        .remedy("`lrfl auth login`");
    SecretResolver::new(store)
        .resolve(&spec)?
        .map(|resolved| resolved.secret)
        .ok_or_else(|| {
            AppError::Auth(format!(
                "not logged in for {account:?} — run `lrfl auth login` or set ${env_var}"
            ))
        })
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use pk_cli_core::CliError;

    use super::*;

    /// A keychain that counts its reads and holds one secret for any account.
    struct Stub {
        secret: Option<&'static str>,
        reads: Cell<u32>,
    }

    impl Stub {
        fn holding(secret: Option<&'static str>) -> Self {
            Self {
                secret,
                reads: Cell::new(0),
            }
        }
    }

    impl KeychainRead for Stub {
        fn service(&self) -> &str {
            "stub"
        }

        fn get(&self, _account: &str) -> Result<Option<Secret>, CliError> {
            self.reads.set(self.reads.get() + 1);
            Ok(self.secret.map(Secret::new))
        }
    }

    // Each test names its own variable: the tests share one process's environment.

    #[test]
    fn the_variable_comes_before_the_keychain() {
        const VAR: &str = "LRFL_TEST_PASSWORD_SET";
        std::env::set_var(VAR, "from-env");
        let store = Stub::holding(Some("stored"));
        assert_eq!(resolve(&store, "user", VAR).unwrap().expose(), "from-env");
        assert_eq!(store.reads.get(), 0);
    }

    #[test]
    fn an_empty_variable_is_an_error_naming_it_not_a_keychain_read() {
        const VAR: &str = "LRFL_TEST_PASSWORD_EMPTY";
        std::env::set_var(VAR, "");
        let store = Stub::holding(Some("stored"));
        let err = resolve(&store, "user", VAR).err().unwrap();
        assert!(err.to_string().contains(VAR), "{err}");
        assert_eq!(store.reads.get(), 0);
    }

    #[test]
    fn an_unset_variable_falls_to_the_keychain_and_nothing_stored_says_so() {
        const VAR: &str = "LRFL_TEST_PASSWORD_UNSET";
        let store = Stub::holding(Some("stored"));
        assert_eq!(resolve(&store, "user", VAR).unwrap().expose(), "stored");
        let err = resolve(&Stub::holding(None), "user", VAR).err().unwrap();
        assert!(matches!(err, AppError::Auth(_)), "{err:?}");
    }
}
