<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/yukimemi/hari/main/assets/logo-dark.svg">
    <img src="https://raw.githubusercontent.com/yukimemi/hari/main/assets/logo.svg" alt="hari — native Git pre-push hooks for jj" width="420">
  </picture>
</p>

# hari

Let `jj git push` run your existing Git **pre-push** hooks.

Hari is a small native Git proxy, not another hook manager. Keep the same
hooks installed by Lefthook, pre-commit, or your own scripts. There is no
Hari hook configuration, shell wrapper, jj alias, or PATH shim.

## Requirements

- Windows, macOS, or Linux.
- jj **0.30.0 or later**, with its Git subprocess transport.
- Git supported by your jj version (jj 0.30.0 documents Git 2.40.4 or later).
- The interpreters and tools required by your hooks, just as for `git push`.
  On Windows, use Git for Windows. Hari does not make POSIX-only hook tools
  portable by itself.

## Install

Install from [crates.io](https://crates.io/crates/hari):

```sh
cargo install hari --locked
```

In a jj repository, enable hooks for that repository:

```sh
hari install
```

Or enable them in jj's user configuration:

```sh
hari install --user
```

Then keep using your usual commands:

```sh
jj git push
jj git push -b feature
git push
```

`git push` is untouched; it already runs Git hooks. Hari does not install or
replace hook scripts. If no pre-push hook is configured, pushes work normally.

### Installation options

```sh
hari install --dir /path/to/hari-bin --jj /path/to/jj --git /path/to/git
```

`--dir` defaults to the platform's local data directory under `hari/bin`.
`--jj` defaults to `jj` on PATH. Without `--git`, Hari preserves the effective
jj `git.executable-path`, resolving an existing Hari proxy back to its original
Git during reinstall.

Installation creates `hari-git` (`hari-git.exe` on Windows) and an adjacent
`hari-git.json` ownership record, then registers the helper using jj's native
`git.executable-path` setting. The JSON records the original executable; it is
not a hook configuration file. Repeat `hari install` after upgrading Hari to
refresh the installed helper.

Existing unowned files are not overwritten. A helper directory already using
a different, available Git executable is not repointed: choose another `--dir`,
so other repositories retain their original Git. If the stored Git has vanished
(for example after a package-manager upgrade), recover the shared helper with
`hari install --git /path/to/current/git` using the same `--dir` and scope.
A higher-precedence setting that bypasses the helper is reported as an error.

On Windows, wait for active jj/Git operations to finish before updating the
installed helper; Windows cannot replace an executable while it is running.

To disable Hari, restore the original Git path printed at installation:

```sh
jj config set --repo git.executable-path /path/to/original/git
# Use --user instead if Hari was registered in user scope.
```

Do not delete a helper while another repository or user configuration still
references it.

## Self-update

Hari uses [kaishin](https://github.com/yukimemi/kaishin) for explicit updates
from GitHub Releases:

```sh
hari self-update --check
hari self-update
hari self-update --yes --non-interactive
```

`--check` only reports availability. `--yes` skips confirmation, while
`--non-interactive` refuses to prompt if an update needs confirmation.
There are no background update checks during `hari install` or Git proxy
operations.

After updating Hari, rerun `hari install` with the original `--dir` and scope
(`--user` if applicable) to refresh the copied Git helper. Self-update does not
change jj configuration or replace those helper copies automatically.

Self-update is enabled by default. Package managers can build without it:

```sh
cargo install hari --locked --no-default-features
```

This omits the `self-update` command and its HTTPS/updater dependencies; Git
hook integration remains available. Use the package manager to update that build.

Default source builds inherit kaishin's native TLS build requirements, including
a C toolchain. Prebuilt GitHub release binaries do not need that toolchain to run.

## How it works

jj normally invokes `git push --porcelain --no-verify ...`, deliberately
skipping hooks. Hari removes **only push's `--no-verify` option**, then runs
the original Git with the other arguments unchanged. Other Git commands are
forwarded unchanged, including fetches.

Git itself handles:

- `.git/hooks`, `core.hooksPath`, and any other hook mechanism your Git supports;
- executable permissions, shebangs, and Git for Windows script launching;
- the remote name and URL, and the exact pre-push ref update stream;
- hook rejection, remote advertisements, and jj's force-with-lease safeguards.

No parsing of jj's human-readable dry-run output, second push plan, or separate
`git hook run` invocation is necessary. jj aliases and `--repository` continue
to work because Hari never parses jj's arguments.

## Semantics and limits

- Only **pre-push** is enabled. There is no pre-commit integration.
- Hooks see Git's normal execution context for the repository used by jj.
  Hari does **not** check out the pushed commit, move `HEAD`, or change the
  working tree. As with `git push`, the working tree may differ from what you
  push. In secondary or non-colocated jj workspaces, Git's hook working
  directory can differ from the current jj workspace.
- Hari does not squash automatic hook fixes. Review and commit them yourself.
- `jj git push --dry-run` does not invoke the Git transport or its hooks.
- A jj configuration override of `git.executable-path` can bypass Hari, just as
  `git push --no-verify` can bypass Git hooks. This is not a security boundary.
- Hook diagnostics depend on your jj version. Some versions do not surface all
  hook output, or report `Git push output unfamiliar` because rejected Git
  pushes may omit the porcelain header jj expects. The push still fails and
  the remote is unchanged.

## Development

Bootstrapped with `kata init github.com/yukimemi/pj-presets:rust-cli`.

```sh
cargo build
cargo run --example smoke
cargo make check
```

The smoke example exercises the actual built binary: native hook rejection
must leave a local bare remote unchanged; removing the hook must allow the
same commit to reach it. It requires Git, not jj.

`tests/hooks.rs` uses real Git/jj and local bare remotes with isolated configs.
It covers hook input, rejection, ref creation/update/deletion, custom hook
paths, aliases, reinstall, and lease safety. When jj is unavailable or older
than 0.30.0, the integration tests explicitly skip. Require real jj execution:

```sh
HARI_REQUIRE_JJ=1 cargo test --test hooks -- --nocapture
```

The repo-owned `hooks.yml` CI requires those tests on Windows, macOS, and
Linux against jj 0.30.0 and 0.45.1. Kata-managed CI and release workflows remain
managed by their templates.

## License

MIT
