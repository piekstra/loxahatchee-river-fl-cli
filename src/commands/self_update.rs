//! `self-update` — update lrfl to the latest GitHub release, via the family
//! updater (`pk-cli-selfupdate`): `--check`, `-y/--yes`, `--json`, and the
//! `self-update/v1` DTO all come from the shared crate.

use pk_cli_selfupdate::SelfUpdateArgs;

use crate::commands::Ctx;
use crate::error::AppError;
use crate::update;

pub fn run(ctx: &Ctx, args: &SelfUpdateArgs) -> Result<(), AppError> {
    ctx.log("checking github releases for piekstra/loxahatchee-river-fl-cli…");
    update::updater().run(args, ctx.json, ctx.quiet)
}
