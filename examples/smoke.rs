//! Exercise the release binary and its native Git hook path without requiring JJ.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn checked(command: &mut Command) -> Output {
    let output = command.output().expect("spawn smoke command");
    assert!(
        output.status.success(),
        "{command:?}\nstatus: {}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn git_command(git: &Path, root: &Path, repo: &Path) -> Command {
    let mut command = Command::new(git);
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            command.env_remove(key);
        }
    }
    command
        .current_dir(repo)
        .env("HOME", root.join("home"))
        .env("USERPROFILE", root.join("home"))
        .env("XDG_CONFIG_HOME", root.join("home"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", root.join("gitconfig"))
        .env("GIT_TEMPLATE_DIR", root.join("templates"))
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_AUTHOR_NAME", "Hari Smoke")
        .env("GIT_AUTHOR_EMAIL", "hari@example.invalid")
        .env("GIT_COMMITTER_NAME", "Hari Smoke")
        .env("GIT_COMMITTER_EMAIL", "hari@example.invalid")
        .env("GIT_AUTHOR_DATE", "2025-06-04T12:00:00Z")
        .env("GIT_COMMITTER_DATE", "2025-06-04T12:00:00Z")
        .env("LC_ALL", "C");
    command
}

fn portable(path: &Path) -> String {
    path.to_str().expect("UTF-8 smoke path").replace('\\', "/")
}

fn main() -> anyhow::Result<()> {
    let example = std::env::current_exe()?;
    let profile = example
        .parent()
        .and_then(Path::parent)
        .expect("Cargo example location");
    let binary = profile.join(format!("hari{}", std::env::consts::EXE_SUFFIX));
    assert!(
        binary.is_file(),
        "build the actual hari binary before running smoke: {}",
        binary.display()
    );
    let root = tempfile::Builder::new()
        .prefix("hari release smoke ")
        .tempdir()?;
    for dir in ["home", "templates", "installed bin", "work tree"] {
        fs::create_dir(root.path().join(dir))?;
    }
    fs::write(root.path().join("gitconfig"), "")?;
    let installed = root.path().join("installed bin");
    let installed_hari = installed.join(format!("hari{}", std::env::consts::EXE_SUFFIX));
    fs::copy(&binary, &installed_hari)?;
    let help = checked(Command::new(&installed_hari).arg("--help"));
    assert!(
        String::from_utf8_lossy(&help.stdout).contains("install"),
        "actual installed CLI help"
    );

    let git: PathBuf = dunce::canonicalize(which::which("git")?)?;
    let proxy = installed.join(format!("hari-git{}", std::env::consts::EXE_SUFFIX));
    // Use the same owned helper layout as `hari install`. Release runners need
    // only Git; real JJ configuration/installation is covered by hooks.yml.
    fs::copy(&binary, &proxy)?;
    fs::write(
        installed.join("hari-git.json"),
        serde_json::to_vec(&serde_json::json!({
            "owner": "hari",
            "schema_version": 1,
            "real_git": git,
        }))?,
    )?;
    let repo = root.path().join("work tree");
    let remote = root.path().join("remote.git");
    checked(git_command(&git, root.path(), &repo).args(["init", "--bare", &portable(&remote)]));
    checked(git_command(&git, root.path(), &repo).args(["init", "-b", "main"]));
    fs::write(repo.join("changed.txt"), "release smoke changed path\n")?;
    checked(git_command(&git, root.path(), &repo).args(["add", "changed.txt"]));
    checked(git_command(&git, root.path(), &repo).args([
        "-c",
        "commit.gpgsign=false",
        "commit",
        "-m",
        "smoke",
    ]));
    checked(git_command(&git, root.path(), &repo).args([
        "remote",
        "add",
        "origin",
        &portable(&remote),
    ]));
    let hook = repo.join(".git/hooks/pre-push");
    fs::create_dir_all(hook.parent().expect("native hook directory"))?;
    fs::write(
        &hook,
        "#!/bin/sh\necho HARI_RELEASE_HOOK_REJECTED >&2\nexit 41\n",
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755))?;
    }
    // This is JJ's actual transport shape. The copied native executable must
    // remove --no-verify and let Git reject before touching the bare remote.
    let rejected = git_command(&proxy, root.path(), &repo)
        .args([
            "--git-dir",
            &portable(&repo.join(".git")),
            "push",
            "--porcelain",
            "--no-verify",
            "--",
            "origin",
            "HEAD:refs/heads/smoke",
        ])
        .output()?;
    assert!(
        !rejected.status.success(),
        "proxy must enable the rejecting native hook"
    );
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("HARI_RELEASE_HOOK_REJECTED"));
    let refs = checked(git_command(&git, root.path(), &repo).args([
        "--git-dir",
        &portable(&remote),
        "for-each-ref",
        "--format=%(refname)",
    ]));
    assert!(
        refs.stdout.is_empty(),
        "rejected push must leave remote unchanged"
    );
    fs::remove_file(hook)?;
    checked(git_command(&proxy, root.path(), &repo).args([
        "push",
        "--porcelain",
        "--no-verify",
        "--",
        "origin",
        "HEAD:refs/heads/smoke",
    ]));
    let remote_oid = checked(git_command(&git, root.path(), &repo).args([
        "--git-dir",
        &portable(&remote),
        "rev-parse",
        "refs/heads/smoke",
    ]));
    let local_oid = checked(git_command(&git, root.path(), &repo).args(["rev-parse", "HEAD"]));
    assert_eq!(
        remote_oid.stdout, local_oid.stdout,
        "missing-hook proxy push must transfer the real commit"
    );
    eprintln!(
        "smoke: installed release binary help, native hook rejection, and missing-hook push passed"
    );
    Ok(())
}
