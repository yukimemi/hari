#[cfg(not(windows))]
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use anyhow::{Context, Result, bail, ensure};
use clap::Args;
use serde::{Deserialize, Serialize};

const OWNER: &str = "hari";
const SCHEMA_VERSION: u32 = 1;
const METADATA_NAME: &str = "hari-git.json";

#[cfg(windows)]
const HELPER_NAME: &str = "hari-git.exe";
#[cfg(not(windows))]
const HELPER_NAME: &str = "hari-git";

#[derive(Args)]
pub struct Options {
    /// Directory for the owned Git helper (defaults to the local data directory/hari/bin).
    #[arg(long, value_name = "DIR")]
    pub dir: Option<PathBuf>,

    /// jj executable to configure.
    #[arg(long, value_name = "JJ", default_value = "jj")]
    pub jj: PathBuf,

    /// Real Git executable (defaults to the effective jj git.executable-path).
    #[arg(long, value_name = "GIT")]
    pub git: Option<PathBuf>,

    /// Configure jj for the user instead of the current repository.
    #[arg(long)]
    pub user: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    owner: String,
    schema_version: u32,
    real_git: PathBuf,
}

/// Recognize the installed executable before clap sees Git's arguments.
pub fn is_helper(path: &Path) -> bool {
    path.file_name().is_some_and(|name| {
        #[cfg(windows)]
        {
            name.to_string_lossy().eq_ignore_ascii_case(HELPER_NAME)
        }
        #[cfg(not(windows))]
        {
            name == OsStr::new(HELPER_NAME)
        }
    })
}

/// Load only owned metadata and reject a proxy chain rather than risking recursion.
pub fn real_git_for_proxy(executable: &Path) -> Result<PathBuf> {
    let executable =
        dunce::canonicalize(executable).context("Cannot resolve the hari Git helper executable")?;
    let directory = executable
        .parent()
        .context("The hari Git helper has no parent directory")?;
    let metadata = read_metadata(&directory.join(METADATA_NAME))?;
    let real_git = resolve_executable(&metadata.real_git, "stored real Git")?;
    reject_recursion(&real_git, &executable)?;
    Ok(real_git)
}

pub fn run(options: Options) -> Result<()> {
    let jj = resolve_executable(&options.jj, "jj (install jj or pass --jj PATH)")?;
    let source = dunce::canonicalize(std::env::current_exe()?)
        .context("Cannot resolve the running hari executable")?;
    let directory = match options.dir {
        Some(directory) => directory,
        None => dirs::data_local_dir()
            .context("Cannot find a local data directory; pass --dir DIR")?
            .join("hari")
            .join("bin"),
    };
    fs::create_dir_all(&directory)
        .with_context(|| format!("Cannot create helper directory {}", directory.display()))?;
    let directory = dunce::canonicalize(&directory)
        .with_context(|| format!("Cannot resolve helper directory {}", directory.display()))?;
    let helper = directory.join(HELPER_NAME);
    let metadata_path = directory.join(METADATA_NAME);
    let helper_exists = regular_file_exists(&helper)?;
    let metadata_exists = regular_file_exists(&metadata_path)?;
    ensure!(
        helper_exists == metadata_exists,
        "Refusing to overwrite an unowned or incomplete installation at {}: {} and {} must both be present with valid hari ownership metadata, or both absent. Move the existing files aside or choose another --dir",
        directory.display(),
        HELPER_NAME,
        METADATA_NAME
    );
    let previous = if metadata_exists {
        Some(read_metadata(&metadata_path)?)
    } else {
        None
    };

    let configured_git = match options.git {
        Some(path) => path,
        None => {
            let output = Command::new(&jj)
                .args([
                    "--ignore-working-copy",
                    "config",
                    "get",
                    "git.executable-path",
                ])
                .output()
                .with_context(|| format!("Cannot run jj at {}", jj.display()))?;
            ensure_success(
                &output,
                "Cannot read jj git.executable-path; check your jj configuration or pass --git PATH",
            )?;
            let value = String::from_utf8(output.stdout)
                .context("jj git.executable-path is not UTF-8; pass --git PATH explicitly")?;
            let value = value.trim_end_matches(['\r', '\n']);
            ensure!(
                !value.is_empty(),
                "jj git.executable-path is empty; pass --git PATH"
            );
            PathBuf::from(value)
        }
    };
    let configured_git =
        resolve_executable(&configured_git, "Git (install Git or pass --git PATH)")?;
    let real_git = if is_helper(&configured_git) {
        real_git_for_proxy(&configured_git)
            .context("The configured Git is a hari helper with invalid metadata; pass --git PATH to a real Git executable")?
    } else {
        configured_git
    };
    reject_recursion(&real_git, &source)?;
    reject_recursion(&real_git, &helper)?;
    if let Some(previous) = &previous {
        ensure!(
            previous.real_git == real_git,
            "The helper directory {} already uses Git at {}. Choose another --dir for {} so other repositories keep their original Git",
            directory.display(),
            previous.real_git.display(),
            real_git.display()
        );
    }
    let output = Command::new(&real_git)
        .arg("--version")
        .output()
        .with_context(|| {
            format!(
                "Cannot execute real Git at {}; pass --git PATH",
                real_git.display()
            )
        })?;
    ensure_success(
        &output,
        "Real Git failed its --version command; pass --git PATH",
    )?;
    ensure!(
        output.stdout.starts_with(b"git version "),
        "{} is not a Git executable; pass --git PATH to real Git",
        real_git.display()
    );

    let metadata = Metadata {
        owner: OWNER.to_owned(),
        schema_version: SCHEMA_VERSION,
        real_git,
    };
    let mut metadata_bytes = serde_json::to_vec_pretty(&metadata)?;
    metadata_bytes.push(b'\n');
    // Stage both files before touching an existing owned installation. In particular,
    // copying to a fresh inode permits upgrades while the old helper is running.
    let staged_helper = stage(&directory, "binary", |file| {
        let mut input = File::open(&source)?;
        io::copy(&mut input, file)?;
        file.set_permissions(fs::metadata(&source)?.permissions())?;
        Ok(())
    })?;
    let staged_metadata = stage(&directory, "metadata", |file| {
        file.write_all(&metadata_bytes)?;
        Ok(())
    })?;
    let helper_text = helper.to_str().context(
        "The helper path cannot be represented in jj's UTF-8 configuration; choose another --dir",
    )?;
    let helper_value = serde_json::to_string(helper_text)?;
    publish(&staged_helper.path, &helper, helper_exists)?;
    if let Err(error) = publish(&staged_metadata.path, &metadata_path, metadata_exists) {
        if !helper_exists {
            let _ = fs::remove_file(&helper);
        }
        return Err(error);
    }
    let output = Command::new(&jj)
        .args(["--ignore-working-copy", "config", "set"])
        .arg(if options.user { "--user" } else { "--repo" })
        .arg("git.executable-path")
        .arg(helper_value)
        .output()
        .with_context(|| format!("Cannot run jj at {} to register the helper", jj.display()))?;
    ensure_success(
        &output,
        "Cannot write jj git.executable-path. Check configuration permissions and run from a jj repository, or use --user. The helper is installed; rerun hari install after fixing the configuration",
    )?;
    let effective = Command::new(&jj)
        .args([
            "--ignore-working-copy",
            "config",
            "get",
            "git.executable-path",
        ])
        .output()
        .with_context(|| {
            format!(
                "Cannot run jj at {} to confirm helper registration",
                jj.display()
            )
        })?;
    ensure_success(
        &effective,
        "Cannot read back jj git.executable-path after installation",
    )?;
    let effective = String::from_utf8(effective.stdout)
        .context("The effective jj git.executable-path is not UTF-8 after installation")?;
    let effective = PathBuf::from(effective.trim_end_matches(['\r', '\n']));
    ensure!(
        resolve_executable(&effective, "effective jj Git after installation")? == helper,
        "The effective jj git.executable-path is still {} instead of {}. A higher-precedence repository, workspace, or environment setting overrides this installation; remove that override or install in repository scope",
        effective.display(),
        helper.display()
    );
    println!("Installed {}", helper.display());
    println!("Real Git: {}", metadata.real_git.display());
    println!(
        "Configured jj git.executable-path in {} scope; existing Git pre-push hooks now run for jj git push.",
        if options.user { "user" } else { "repository" }
    );
    Ok(())
}

fn read_metadata(path: &Path) -> Result<Metadata> {
    ensure!(
        regular_file_exists(path)?,
        "Missing hari ownership metadata at {}; rerun hari install with --git PATH",
        path.display()
    );
    let metadata: Metadata = serde_json::from_reader(
        File::open(path).with_context(|| format!("Cannot read {}", path.display()))?,
    )
    .with_context(|| format!("Invalid hari ownership metadata at {}", path.display()))?;
    ensure!(
        metadata.owner == OWNER && metadata.schema_version == SCHEMA_VERSION,
        "Unrecognized ownership or metadata schema at {}; refusing to use or overwrite it",
        path.display()
    );
    ensure!(
        metadata.real_git.is_absolute(),
        "Real Git in {} must be an absolute path",
        path.display()
    );
    Ok(metadata)
}

fn regular_file_exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            ensure!(
                metadata.file_type().is_file(),
                "Refusing to use or overwrite non-regular file {}",
                path.display()
            );
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).with_context(|| format!("Cannot inspect {}", path.display())),
    }
}

fn resolve_executable(path: &Path, description: &str) -> Result<PathBuf> {
    let resolved = which::which(path)
        .with_context(|| format!("Cannot find executable {description}: {}", path.display()))?;
    dunce::canonicalize(&resolved).with_context(|| {
        format!(
            "Cannot resolve executable {description}: {}",
            resolved.display()
        )
    })
}

fn reject_recursion(real_git: &Path, helper: &Path) -> Result<()> {
    ensure!(
        !is_helper(real_git) && real_git != helper,
        "Refusing recursive Git executable {}; pass --git PATH to a real Git executable",
        real_git.display()
    );
    Ok(())
}

fn ensure_success(output: &Output, message: &str) -> Result<()> {
    if !output.status.success() {
        bail!(
            "{message}\n{}{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
    }
    Ok(())
}

struct StagedFile {
    path: PathBuf,
}

impl Drop for StagedFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn stage(
    directory: &Path,
    kind: &str,
    write: impl FnOnce(&mut File) -> Result<()>,
) -> Result<StagedFile> {
    let mut sequence = 0_u32;
    let (staged, mut file) = loop {
        let path = directory.join(format!(
            ".hari-{}-{kind}-{sequence}.tmp",
            std::process::id()
        ));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => break (StagedFile { path }, file),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                sequence = sequence
                    .checked_add(1)
                    .context("Too many hari staging files")?;
            }
            Err(error) => return Err(error).context("Cannot stage the hari installation"),
        }
    };
    write(&mut file).context("Cannot write the staged hari installation")?;
    file.sync_all()
        .context("Cannot sync the staged hari installation")?;
    Ok(staged)
}

fn publish(staged: &Path, destination: &Path, owned: bool) -> Result<()> {
    if !owned {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)
            .with_context(|| {
                format!(
                    "Cannot install {} without overwriting existing files",
                    destination.display()
                )
            })?;
        let result = (|| -> Result<()> {
            let mut input = File::open(staged)?;
            io::copy(&mut input, &mut file)?;
            file.set_permissions(fs::metadata(staged)?.permissions())?;
            file.sync_all()?;
            Ok(())
        })();
        drop(file);
        if let Err(error) = result {
            let _ = fs::remove_file(destination);
            return Err(error).with_context(|| {
                format!("Cannot write installation file {}", destination.display())
            });
        }
    } else {
        fs::rename(staged, destination).with_context(|| {
            format!(
                "Cannot replace owned installation file {}",
                destination.display()
            )
        })?;
    }
    Ok(())
}
