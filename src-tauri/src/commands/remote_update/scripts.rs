//! Shared Bash operations; adapters supply data only.
use crate::core::shell;

pub(super) const TRANSFER: &str = include_str!("../../../../shell/transfer.sh");
const BACKUP: &str = include_str!("../../../../shell/backup.sh");
const APPLY: &str = include_str!("../../../../shell/compose-update.sh");
pub(super) const READINESS: &str = include_str!("../../../../shell/readiness.sh");

pub(super) fn transfer(action: &str, target: &str, stage: &str) -> Result<String, String> {
    shell::script(TRANSFER, &[action, target, stage])
}

pub(super) fn backup(install: &str, targets: &[String]) -> Result<String, String> {
    let mut args = vec![install];
    args.extend(targets.iter().map(String::as_str));
    shell::script(BACKUP, &args)
}

pub(super) fn apply(
    deploy: &str,
    compose: &str,
    config: bool,
    restart: &[String],
    wait: &[String],
) -> Result<String, String> {
    let count = restart.len().to_string();
    let mut args = vec![deploy, compose, if config { "yes" } else { "no" }, &count];
    args.extend(restart.iter().map(String::as_str));
    args.extend(wait.iter().map(String::as_str));
    shell::script(&format!("{APPLY}\n{READINESS}\ncompose ps\n"), &args)
}
