//! Real local transports: no mocked Git/JJ, network, or developer configuration.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const ZERO: &str = "0000000000000000000000000000000000000000";
const CAPTURE_HOOK: &str =
    "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$HARI_CAPTURE.args\"\ncat > \"$HARI_CAPTURE.stdin\"\n";

struct Fixture {
    root: tempfile::TempDir,
    repo: PathBuf,
    remote: PathBuf,
    git: PathBuf,
    jj: PathBuf,
    allow_new: bool,
}

fn checked(command: &mut Command) -> Output {
    let output = command.output().expect("spawn test command");
    assert!(
        output.status.success(),
        "{command:?}\nstatus: {}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn text(output: &Output) -> String {
    String::from_utf8(output.stdout.clone())
        .expect("UTF-8 command output")
        .trim()
        .to_owned()
}

fn portable(path: &Path) -> String {
    path.to_str().expect("UTF-8 test path").replace('\\', "/")
}

fn hook(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().expect("hook directory")).expect("create hook directory");
    fs::write(path, body).expect("write hook with LF line endings");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("make hook executable");
    }
}

impl Fixture {
    fn new() -> Option<Self> {
        let jj = match which::which("jj") {
            Ok(path) => path,
            Err(error) => {
                assert!(
                    std::env::var_os("HARI_REQUIRE_JJ").is_none(),
                    "HARI_REQUIRE_JJ is set but jj is unavailable: {error}"
                );
                eprintln!("SKIP hari Git/JJ integration: jj >= 0.30.0 is not on PATH ({error})");
                return None;
            }
        };
        let git = which::which("git").expect("Git is required for hook integration");
        let root = tempfile::Builder::new()
            .prefix("hari hooks ")
            .tempdir()
            .expect("create isolated sandbox");
        for dir in ["home", "config", "templates"] {
            fs::create_dir(root.path().join(dir)).expect("create isolated config directory");
        }
        fs::write(root.path().join("gitconfig"), "").expect("empty Git global config");
        fs::write(
            root.path().join("jj.toml"),
            "[user]\nname = 'Hari Test'\nemail = 'hari@example.invalid'\n[signing]\nbehavior = 'drop'\n[ui]\ncolor = 'never'\npaginate = 'never'\n",
        )
        .expect("isolated JJ config");
        let mut fixture = Self {
            repo: root.path().join("work tree"),
            remote: root.path().join("remote.git"),
            root,
            git,
            jj,
            allow_new: false,
        };
        let version = text(&checked(fixture.command(&fixture.jj).arg("--version")));
        let numbers: Vec<u32> = version
            .split_whitespace()
            .nth(1)
            .expect("jj version number")
            .split(['.', '-'])
            .take(3)
            .map(|part| part.parse().expect("numeric jj version"))
            .collect();
        if numbers.as_slice() < [0, 30, 0].as_slice() {
            assert!(
                std::env::var_os("HARI_REQUIRE_JJ").is_none(),
                "jj >= 0.30.0 required, found {version}"
            );
            eprintln!(
                "SKIP hari Git/JJ integration: {version} predates the subprocess-only backend"
            );
            return None;
        }
        checked(fixture.command(&fixture.git).args([
            "init",
            "--bare",
            "--object-format=sha1",
            &portable(&fixture.remote),
        ]));
        checked(fixture.command(&fixture.git).args([
            "init",
            "--object-format=sha1",
            "-b",
            "main",
            &portable(&fixture.repo),
        ]));
        fixture.git(&["config", "user.name", "Hari Test"]);
        fixture.git(&["config", "user.email", "hari@example.invalid"]);
        fixture.git(&["config", "commit.gpgsign", "false"]);
        fixture.git(&["config", "tag.gpgsign", "false"]);
        fixture.git(&["config", "core.autocrlf", "false"]);
        fs::write(fixture.repo.join("file.txt"), "initial\n").expect("initial file");
        fixture.git(&["add", "file.txt"]);
        fixture.git(&["commit", "-m", "initial"]);
        fixture.git(&["remote", "add", "origin", &portable(&fixture.remote)]);
        fixture.git(&["push", "origin", "main"]);
        checked(fixture.command(&fixture.jj).args([
            "git",
            "init",
            "--colocate",
            &portable(&fixture.repo),
        ]));
        fixture.allow_new = text(&fixture.jj(&["git", "push", "--help"])).contains("--allow-new");
        Some(fixture)
    }

    fn command(&self, executable: impl AsRef<OsStr>) -> Command {
        let mut command = Command::new(executable);
        command.current_dir(self.root.path());
        // Environment changes are per child, so parallel tests cannot race and no
        // administrative command can reach the developer's actual JJ/Git config.
        for (key, _) in std::env::vars_os() {
            let name = key.to_string_lossy();
            if name.starts_with("JJ_") || name.starts_with("GIT_") {
                command.env_remove(key);
            }
        }
        command
            .env("HOME", self.root.path().join("home"))
            .env("USERPROFILE", self.root.path().join("home"))
            .env("XDG_CONFIG_HOME", self.root.path().join("config"))
            .env("JJ_CONFIG", self.root.path().join("jj.toml"))
            .env("JJ_TIMESTAMP", "2025-06-04T12:00:00Z")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.root.path().join("gitconfig"))
            .env("GIT_TEMPLATE_DIR", self.root.path().join("templates"))
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_AUTHOR_DATE", "1749038400 +0000")
            .env("GIT_COMMITTER_DATE", "1749038400 +0000")
            .env("HARI_CAPTURE", portable(&self.root.path().join("capture")))
            .env("LC_ALL", "C");
        command
    }

    fn git(&self, args: &[&str]) -> Output {
        checked(self.command(&self.git).current_dir(&self.repo).args(args))
    }

    fn jj(&self, args: &[&str]) -> Output {
        checked(self.command(&self.jj).current_dir(&self.repo).args(args))
    }

    fn push(&self, args: &[&str]) -> Output {
        let mut command = self.command(&self.jj);
        command.current_dir(&self.repo).args(["git", "push"]);
        // 0.30 requires this ordinary native option; newer jj removed it.
        if self.allow_new && args.contains(&"--bookmark") {
            command.arg("--allow-new");
        }
        command
            .args(args)
            .output()
            .expect("run unchanged jj git push")
    }

    fn install(&self, explicit_git: bool, user: bool) {
        let mut command = self.command(env!("CARGO_BIN_EXE_hari"));
        command
            .current_dir(&self.repo)
            .arg("install")
            .arg("--dir")
            .arg(self.install_dir())
            .arg("--jj")
            .arg(&self.jj);
        if explicit_git {
            command.arg("--git").arg(&self.git);
        }
        if user {
            command.arg("--user");
        }
        checked(&mut command);
    }

    fn install_dir(&self) -> PathBuf {
        self.root.path().join("installed bin")
    }

    fn proxy(&self) -> PathBuf {
        self.install_dir().join(if cfg!(windows) {
            "hari-git.exe"
        } else {
            "hari-git"
        })
    }

    fn change(&self, content: &str) -> String {
        fs::write(self.repo.join("file.txt"), format!("{content}\n")).expect("write changed path");
        self.jj(&["describe", "-m", content]);
        let oid = text(&self.jj(&["log", "--no-graph", "-r", "@", "-T", "commit_id"]));
        self.jj(&["new"]);
        oid
    }

    fn bookmark(&self, name: &str, oid: &str) {
        self.jj(&["bookmark", "set", name, "-r", oid]);
    }

    fn default_hook(&self, body: &str) {
        hook(&self.repo.join(".git/hooks/pre-push"), body);
    }

    fn capture(&self, suffix: &str) -> String {
        fs::read_to_string(self.root.path().join(format!("capture.{suffix}")))
            .expect("native Git ran pre-push hook")
    }

    fn assert_input(&self, expected: &[String]) {
        let mut actual: Vec<String> = self
            .capture("stdin")
            .split_inclusive('\n')
            .map(str::to_owned)
            .collect();
        let mut expected = expected.to_vec();
        actual.sort();
        expected.sort();
        assert_eq!(
            actual, expected,
            "exact native pre-push stdin, including final LF"
        );
        assert_eq!(
            self.capture("args"),
            format!("origin\n{}\n", portable(&self.remote)),
            "native hook remote name and URL"
        );
    }

    fn remote_refs(&self) -> String {
        text(&checked(self.command(&self.git).args([
            "--git-dir",
            &portable(&self.remote),
            "show-ref",
        ])))
    }

    fn remote_oid(&self, name: &str) -> String {
        text(&checked(self.command(&self.git).args([
            "--git-dir",
            &portable(&self.remote),
            "rev-parse",
            name,
        ])))
    }
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn installed_jj_and_unchanged_git_push_share_a_rejecting_native_hook() {
    let Some(f) = Fixture::new() else { return };
    f.install(true, false);
    let oid = f.change("rejected content");
    f.bookmark("topic", &oid);
    f.default_hook(&format!(
        "{CAPTURE_HOOK}echo HARI_HOOK_REJECTED >&2\nexit 41\n"
    ));
    let before = f.remote_refs();
    let head = text(&f.git(&["rev-parse", "HEAD"]));
    let status = text(&f.git(&["status", "--porcelain"]));
    let jj = f.push(&["--bookmark", "topic"]);
    assert!(!jj.status.success(), "hook rejection must fail jj push");
    f.assert_input(&[format!("{oid} {oid} refs/heads/topic {ZERO}\n")]);
    assert_eq!(f.remote_refs(), before);
    assert_eq!(text(&f.git(&["rev-parse", "HEAD"])), head);
    assert_eq!(text(&f.git(&["status", "--porcelain"])), status);

    let git = f
        .command(&f.git)
        .current_dir(&f.repo)
        .args(["push", "origin", "topic"])
        .output()
        .expect("run unchanged git push");
    assert!(
        !git.status.success(),
        "same hook must reject native Git push"
    );
    assert!(String::from_utf8_lossy(&git.stderr).contains("HARI_HOOK_REJECTED"));
    f.assert_input(&[format!("refs/heads/topic {oid} refs/heads/topic {ZERO}\n")]);
    assert_eq!(f.remote_refs(), before);
}

#[test]
fn jj_multiple_bookmarks_create_update_delete_with_exact_native_input() {
    let Some(f) = Fixture::new() else { return };
    f.install(true, false);
    f.default_hook(CAPTURE_HOOK);
    let first = f.change("first shared commit");
    f.bookmark("alpha", &first);
    f.bookmark("beta", &first);
    assert_success(&f.push(&["--bookmark", "alpha", "--bookmark", "beta"]));
    f.assert_input(&[
        format!("{first} {first} refs/heads/alpha {ZERO}\n"),
        format!("{first} {first} refs/heads/beta {ZERO}\n"),
    ]);
    assert_eq!(f.remote_oid("refs/heads/alpha"), first);
    assert_eq!(f.remote_oid("refs/heads/beta"), first);

    let second = f.change("second shared commit");
    f.bookmark("alpha", &second);
    f.bookmark("beta", &second);
    assert_success(&f.push(&["--bookmark", "alpha", "--bookmark", "beta"]));
    f.assert_input(&[
        format!("{second} {second} refs/heads/alpha {first}\n"),
        format!("{second} {second} refs/heads/beta {first}\n"),
    ]);
    assert_eq!(f.remote_oid("refs/heads/alpha"), second);
    assert_eq!(f.remote_oid("refs/heads/beta"), second);

    f.jj(&["bookmark", "delete", "beta"]);
    assert_success(&f.push(&["--deleted"]));
    f.assert_input(&[format!("(delete) {ZERO} refs/heads/beta {second}\n")]);
    assert!(!f.remote_refs().contains("refs/heads/beta"));
    assert_eq!(f.remote_oid("refs/heads/alpha"), second);
}

#[test]
fn unchanged_native_git_push_supports_branch_and_tag_ref_updates() {
    let Some(f) = Fixture::new() else { return };
    f.install(true, false);
    f.default_hook(CAPTURE_HOOK);
    let first = f.change("native first");
    f.git(&["update-ref", "refs/heads/native", &first]);
    f.git(&["push", "origin", "native"]);
    f.assert_input(&[format!(
        "refs/heads/native {first} refs/heads/native {ZERO}\n"
    )]);
    assert_eq!(f.remote_oid("refs/heads/native"), first);
    let second = f.change("native second");
    f.git(&["update-ref", "refs/heads/native", &second]);
    f.git(&["push", "origin", "native"]);
    f.assert_input(&[format!(
        "refs/heads/native {second} refs/heads/native {first}\n"
    )]);
    assert_eq!(f.remote_oid("refs/heads/native"), second);
    f.git(&["tag", "-a", "native-tag", "-m", "native tag", &second]);
    let tag_oid = text(&f.git(&["rev-parse", "refs/tags/native-tag"]));
    f.git(&["push", "origin", "refs/tags/native-tag"]);
    f.assert_input(&[format!(
        "refs/tags/native-tag {tag_oid} refs/tags/native-tag {ZERO}\n"
    )]);
    assert_eq!(f.remote_oid("refs/tags/native-tag"), tag_oid);
    f.git(&[
        "push",
        "origin",
        ":refs/heads/native",
        ":refs/tags/native-tag",
    ]);
    f.assert_input(&[
        format!("(delete) {ZERO} refs/heads/native {second}\n"),
        format!("(delete) {ZERO} refs/tags/native-tag {tag_oid}\n"),
    ]);
    assert!(!f.remote_refs().contains("refs/heads/native"));
    assert!(!f.remote_refs().contains("refs/tags/native-tag"));
}

#[test]
fn core_hooks_path_is_discovered_by_git_for_both_push_commands() {
    let Some(f) = Fixture::new() else { return };
    f.install(true, false);
    f.default_hook("#!/bin/sh\necho HARI_WRONG_DEFAULT >&2\nexit 99\n");
    let custom = f.root.path().join("custom hooks with spaces");
    hook(&custom.join("pre-push"), CAPTURE_HOOK);
    f.git(&["config", "core.hooksPath", &portable(&custom)]);
    let oid = f.change("custom hook");
    f.bookmark("custom", &oid);
    assert_success(&f.push(&["--bookmark", "custom"]));
    f.assert_input(&[format!("{oid} {oid} refs/heads/custom {ZERO}\n")]);
    f.git(&["push", "origin", "custom:refs/heads/native-custom"]);
    f.assert_input(&[format!(
        "refs/heads/custom {oid} refs/heads/native-custom {ZERO}\n"
    )]);
    assert_eq!(f.remote_oid("refs/heads/native-custom"), oid);
}

#[test]
fn jj_alias_repository_flag_named_push_and_dry_run_remain_native() {
    let Some(f) = Fixture::new() else { return };
    f.install(true, false);
    f.default_hook(CAPTURE_HOOK);
    f.jj(&[
        "config",
        "set",
        "--user",
        "aliases.ship",
        "[\"git\", \"push\"]",
    ]);
    let oid = f.change("aliased named push");
    let named = format!("shipped={oid}");
    let before = f.remote_refs();
    checked(f.command(&f.jj).args([
        "--repository",
        &portable(&f.repo),
        "ship",
        "--named",
        &named,
        "--dry-run",
    ]));
    assert_eq!(f.remote_refs(), before);
    assert!(
        !f.root.path().join("capture.stdin").exists(),
        "jj dry-run must not invoke transport or hooks"
    );
    checked(f.command(&f.jj).args([
        "--repository",
        &portable(&f.repo),
        "ship",
        "--named",
        &named,
    ]));
    f.assert_input(&[format!("{oid} {oid} refs/heads/shipped {ZERO}\n")]);
    assert_eq!(f.remote_oid("refs/heads/shipped"), oid);
}

#[test]
fn missing_hook_and_reinstall_keep_real_git_and_successful_pushes() {
    let Some(f) = Fixture::new() else { return };
    f.jj(&[
        "config",
        "set",
        "--repo",
        "git.executable-path",
        &portable(&f.git),
    ]);
    f.install(false, false);
    let metadata = fs::read(f.install_dir().join("hari-git.json")).expect("installation metadata");
    f.install(false, false);
    assert_eq!(
        fs::read(f.install_dir().join("hari-git.json")).expect("reinstalled metadata"),
        metadata
    );
    let metadata: serde_json::Value = serde_json::from_slice(&metadata).expect("metadata JSON");
    assert_eq!(
        dunce::canonicalize(Path::new(
            metadata["real_git"].as_str().expect("real Git path")
        ))
        .expect("canonical real Git"),
        dunce::canonicalize(&f.git).expect("canonical original Git")
    );
    let configured = text(&f.jj(&[
        "--ignore-working-copy",
        "config",
        "get",
        "git.executable-path",
    ]));
    assert_eq!(
        dunce::canonicalize(configured).expect("configured proxy"),
        dunce::canonicalize(f.proxy()).expect("installed proxy")
    );
    let version = text(&checked(f.command(f.proxy()).arg("--version")));
    assert!(
        version.starts_with("git version "),
        "proxy must dispatch before clap"
    );
    assert!(!f.repo.join(".git/hooks/pre-push").exists());
    let oid = f.change("missing hook succeeds");
    f.bookmark("no-hook", &oid);
    assert_success(&f.push(&["--bookmark", "no-hook"]));
    assert_eq!(f.remote_oid("refs/heads/no-hook"), oid);
    f.git(&["push", "origin", "no-hook:refs/heads/native-no-hook"]);
    assert_eq!(f.remote_oid("refs/heads/native-no-hook"), oid);
}

#[test]
fn user_install_is_isolated_and_also_enables_repo_push_hooks() {
    let Some(f) = Fixture::new() else { return };
    f.install(true, true);
    assert!(
        !fs::read_to_string(f.repo.join(".jj/repo/config.toml"))
            .unwrap_or_default()
            .contains("executable-path")
    );
    assert!(
        fs::read_to_string(f.root.path().join("jj.toml"))
            .expect("isolated user config")
            .contains("executable-path")
    );
    f.default_hook(CAPTURE_HOOK);
    let oid = f.change("user-scoped installation");
    f.bookmark("user-scope", &oid);
    assert_success(&f.push(&["--bookmark", "user-scope"]));
    f.assert_input(&[format!("{oid} {oid} refs/heads/user-scope {ZERO}\n")]);
}

#[test]
fn stale_remote_lease_is_not_bypassed_by_enabling_hooks() {
    let Some(f) = Fixture::new() else { return };
    f.install(true, false);
    f.default_hook(CAPTURE_HOOK);
    let first = f.change("lease first");
    f.bookmark("lease", &first);
    assert_success(&f.push(&["--bookmark", "lease"]));
    let external = f.change("external writer");
    // Copy the object without moving the tracked lease ref, then simulate a
    // second writer changing the bare remote behind JJ's last observation.
    f.git(&["push", "origin", &format!("{external}:refs/heads/external")]);
    checked(f.command(&f.git).args([
        "--git-dir",
        &portable(&f.remote),
        "update-ref",
        "refs/heads/lease",
        &external,
    ]));
    let local = f.change("local writer");
    f.bookmark("lease", &local);
    let before = f.remote_refs();
    let output = f.push(&["--bookmark", "lease"]);
    assert!(
        !output.status.success(),
        "stale lease must still reject the push"
    );
    assert_eq!(f.remote_refs(), before);
    assert_eq!(f.remote_oid("refs/heads/lease"), external);
}

#[test]
fn jj_tag_push_uses_native_hook_when_supported() {
    let Some(f) = Fixture::new() else { return };
    if !text(&f.jj(&["git", "push", "--help"])).contains("--tag") {
        eprintln!(
            "SKIP jj tag push: installed jj has no native git push --tag support (native Git tags are covered separately)"
        );
        return;
    }
    f.install(true, false);
    f.default_hook(CAPTURE_HOOK);
    let oid = f.change("JJ tag target");
    f.jj(&["tag", "set", "jj-tag", "-r", &oid]);
    assert_success(&f.push(&["--tag", "jj-tag"]));
    f.assert_input(&[format!("{oid} {oid} refs/tags/jj-tag {ZERO}\n")]);
    assert_eq!(f.remote_oid("refs/tags/jj-tag"), oid);
}

#[test]
fn shared_install_directory_rejects_a_different_real_git_without_changes() {
    let Some(f) = Fixture::new() else { return };
    f.install(true, false);
    let original_metadata =
        fs::read(f.install_dir().join("hari-git.json")).expect("original metadata");
    let original_helper = fs::read(f.proxy()).expect("original helper");
    let original_config = text(&f.jj(&["config", "get", "git.executable-path"]));
    let second = f.root.path().join("second repository");
    checked(
        f.command(&f.git)
            .args(["init", "-b", "main", &portable(&second)]),
    );
    checked(
        f.command(&f.jj)
            .args(["git", "init", "--colocate", &portable(&second)]),
    );
    let other_git = f
        .root
        .path()
        .join(format!("other-git{}", std::env::consts::EXE_SUFFIX));
    fs::copy(&f.git, &other_git).expect("create distinct real Git executable path");
    checked(f.command(&f.jj).current_dir(&second).args([
        "config",
        "set",
        "--repo",
        "git.executable-path",
        &portable(&other_git),
    ]));
    let second_config = text(&checked(f.command(&f.jj).current_dir(&second).args([
        "config",
        "get",
        "git.executable-path",
    ])));
    let output = f
        .command(env!("CARGO_BIN_EXE_hari"))
        .current_dir(&second)
        .arg("install")
        .arg("--dir")
        .arg(f.install_dir())
        .arg("--jj")
        .arg(&f.jj)
        .arg("--git")
        .arg(&other_git)
        .output()
        .expect("attempt conflicting shared installation");
    assert!(
        !output.status.success(),
        "another repo must not repoint an already-owned proxy to different Git"
    );
    assert_eq!(
        fs::read(f.install_dir().join("hari-git.json")).expect("preserved metadata"),
        original_metadata
    );
    assert_eq!(
        fs::read(f.proxy()).expect("preserved helper"),
        original_helper
    );
    assert_eq!(
        text(&f.jj(&["config", "get", "git.executable-path"])),
        original_config
    );
    assert_eq!(
        text(&checked(f.command(&f.jj).current_dir(&second).args([
            "config",
            "get",
            "git.executable-path"
        ]),)),
        second_config
    );
}

#[cfg(unix)]
#[test]
fn reinstall_recovers_when_the_stored_real_git_has_vanished() {
    let Some(f) = Fixture::new() else { return };
    // A relocated Git binary may not find its own runtime, so stand in for a
    // versioned install path with a removable wrapper around the real Git.
    let vanishing_git = f.root.path().join("vanishing-git");
    fs::write(
        &vanishing_git,
        format!("#!/bin/sh\nexec '{}' \"$@\"\n", portable(&f.git)),
    )
    .expect("create a removable real Git executable");
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&vanishing_git, fs::Permissions::from_mode(0o755))
            .expect("make Git wrapper executable");
    }
    f.jj(&[
        "config",
        "set",
        "--repo",
        "git.executable-path",
        &portable(&vanishing_git),
    ]);
    f.install(false, false);
    fs::remove_file(&vanishing_git).expect("simulate a Git upgrade removing the stored path");
    assert!(
        !f.command(f.proxy())
            .arg("--version")
            .status()
            .expect("run proxy")
            .success(),
        "the proxy cannot run a vanished Git"
    );
    f.install(true, false);
    let metadata: serde_json::Value = serde_json::from_slice(
        &fs::read(f.install_dir().join("hari-git.json")).expect("repointed metadata"),
    )
    .expect("metadata JSON");
    assert_eq!(
        dunce::canonicalize(Path::new(
            metadata["real_git"].as_str().expect("real Git path")
        ))
        .expect("canonical real Git"),
        dunce::canonicalize(&f.git).expect("canonical original Git")
    );
    f.default_hook(CAPTURE_HOOK);
    let oid = f.change("recovered after Git upgrade");
    f.bookmark("recovered", &oid);
    assert_success(&f.push(&["--bookmark", "recovered"]));
    f.assert_input(&[format!("{oid} {oid} refs/heads/recovered {ZERO}\n")]);
}
