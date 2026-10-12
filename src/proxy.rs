//! Transparent Git subprocess proxy; only pre-push bypass is changed.

use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, ensure};

/// Run the original Git, re-enabling native pre-push hooks for pushes.
///
/// Arguments remain OS strings, and options after push's `--` separator
/// are never interpreted. Other Git commands are forwarded unchanged.
/// On Unix this replaces the process, preserving Git's signals and status.
pub fn run(real_git: &Path, args: &[OsString]) -> Result<i32> {
    let real_git = dunce::canonicalize(real_git)
        .with_context(|| format!("cannot locate original Git at {}", real_git.display()))?;
    let current = dunce::canonicalize(std::env::current_exe()?)?;
    ensure!(real_git != current, "hari's Git proxy cannot invoke itself");

    let mut command = Command::new(&real_git);
    if let Some(push) = push_command(args) {
        command.args(&args[..=push]);
        let mut after_separator = false;
        for arg in &args[push + 1..] {
            if !after_separator && arg == OsStr::new("--no-verify") {
                continue;
            }
            if arg == OsStr::new("--") {
                after_separator = true;
            }
            command.arg(arg);
        }
    } else {
        command.args(args);
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        Err(command.exec()).with_context(|| format!("cannot execute {}", real_git.display()))
    }
    #[cfg(not(unix))]
    {
        let status = command
            .status()
            .with_context(|| format!("cannot execute {}", real_git.display()))?;
        Ok(status.code().unwrap_or(1))
    }
}

// Git's global options can contain the word `push` as their VALUE. Find
// the actual subcommand instead of searching all arguments for that word.
fn push_command(args: &[OsString]) -> Option<usize> {
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        if arg == OsStr::new("--") {
            return args
                .get(index + 1)
                .filter(|arg| *arg == OsStr::new("push"))
                .map(|_| index + 1);
        }
        if matches!(
            arg.to_str(),
            Some(
                "-c" | "-C"
                    | "--git-dir"
                    | "--work-tree"
                    | "--namespace"
                    | "--config-env"
                    | "--super-prefix"
            )
        ) {
            // Missing values are left for Git's own argument validation.
            index += 2;
            continue;
        }
        if matches!(arg.to_str(), Some("--help" | "--version" | "--exec-path")) {
            return None;
        }
        if arg.as_encoded_bytes().starts_with(b"-") {
            index += 1;
            continue;
        }
        return (arg == OsStr::new("push")).then_some(index);
    }
    None
}
