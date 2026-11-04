//! `config` — manage the saved default account number (not a secret).

use crate::acct::AccountId;
use crate::cli::ConfigAction;
use crate::commands::Ctx;
use crate::config;
use crate::error::AppError;

pub fn run(ctx: &Ctx, action: &ConfigAction) -> Result<(), AppError> {
    match action {
        ConfigAction::SetAccount { account } => {
            // Validate before saving so we never persist garbage.
            let id = AccountId::parse(account)?;
            config::save_default_account(&id.dashed())
                .map_err(|e| AppError::Other(format!("saving default account: {e}")))?;
            if ctx.json {
                println!(
                    "{}",
                    serde_json::json!({ "default_account": id.dashed(), "saved": true })
                );
            } else if !ctx.quiet {
                println!("✓ default account set to {}", id.dashed());
            }
        }
        ConfigAction::Clear => {
            let removed = config::clear_default_account().map_err(|e| {
                crate::error::AppError::Other(format!("removing the default account: {e}"))
            })?;
            if ctx.json {
                println!("{}", serde_json::json!({ "cleared": removed }));
            } else if !ctx.quiet {
                println!(
                    "{}",
                    if removed {
                        "✓ cleared the default account"
                    } else {
                        "no default account was set"
                    }
                );
            }
        }
        ConfigAction::Show => {
            let current = config::load_default_account();
            if ctx.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "default_account": current,
                        "path": config::config_path_display(),
                    })
                );
            } else {
                match current {
                    Some(a) => println!("default account: {a}"),
                    None => println!("no default account set"),
                }
                println!("stored at: {}", config::config_path_display());
            }
        }
    }
    Ok(())
}
