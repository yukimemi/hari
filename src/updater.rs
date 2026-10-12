//! Explicit self-update; Git proxy operations never start an updater.

use anyhow::Result;
use kaishin::{KaishinOptions, UpdateOptions};

pub fn run(yes: bool, check_only: bool, non_interactive: bool) -> Result<()> {
    let options = KaishinOptions::new("yukimemi", "hari", "hari", env!("CARGO_PKG_VERSION"))
        .crate_name(env!("CARGO_PKG_NAME"));
    let update = UpdateOptions::new()
        .yes(yes)
        .check_only(check_only)
        .non_interactive(non_interactive);
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(kaishin::run_self_update(&options, update))
}
