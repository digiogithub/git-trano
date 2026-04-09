use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use clap::{ArgAction, Parser};

#[derive(Debug, Parser)]
#[command(
    name = "git-trano",
    version,
    about = "Git plugin for Capistrano-style deployments",
    long_about = "Git plugin to deploy branch/tag from the current repository into a Capistrano-like layout:\n\
                  - releases/<timestamp>\n\
                  - current (symlink)\n\
                  - shared\n\
                  Supports rollback with --revert and shared paths with --shared."
)]
struct Cli {
    /// Branch to deploy (mutually exclusive with --tag and --revert)
    #[arg(short = 'b', long = "branch", conflicts_with_all = ["tag", "revert"])]
    branch: Option<String>,

    /// Tag to deploy (mutually exclusive with --branch and --revert)
    #[arg(short = 't', long = "tag", conflicts_with_all = ["branch", "revert"])]
    tag: Option<String>,

    /// Revert current to the previous release (mutually exclusive with --branch/--tag)
    #[arg(short = 'r', long = "revert", action = ArgAction::SetTrue, conflicts_with_all = ["branch", "tag"])]
    revert: bool,

    /// Deploy base path (e.g. /www/folder)
    #[arg(short = 'p', long = "path")]
    deploy_path: PathBuf,

    /// Number of releases to keep (default: 3)
    #[arg(short = 'k', long = "keep", default_value_t = 3)]
    keep: usize,

    /// Shared paths (repeatable), e.g.:
    /// --shared node_modules --shared vendor/subfolder
    #[arg(long = "shared")]
    shared: Vec<String>,
}

#[derive(Debug)]
struct DeployLayout {
    releases: PathBuf,
    shared: PathBuf,
    current: PathBuf,
}

#[derive(Debug)]
struct DeployContext {
    repo_root: PathBuf,
    deploy_base: PathBuf,
}

#[derive(Debug)]
enum RefTarget {
    Branch(String),
    Tag(String),
}

impl RefTarget {
    fn describe(&self) -> String {
        match self {
            RefTarget::Branch(b) => format!("branch:{b}"),
            RefTarget::Tag(t) => format!("tag:{t}"),
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    validate_cli(&cli)?;

    ensure_git_repo()?;

    let ctx = build_context(&cli.deploy_path)?;
    let layout = prepare_layout(&ctx.deploy_base)?;

    if cli.revert {
        do_revert(&layout)?;
        println!("Revert completed. 'current' now points to the previous release.");
        return Ok(());
    }

    let reference = match (&cli.branch, &cli.tag) {
        (Some(branch), None) => RefTarget::Branch(branch.clone()),
        (None, Some(tag)) => RefTarget::Tag(tag.clone()),
        _ => bail!("You must provide exactly one of: --branch or --tag"),
    };

    deploy(&ctx, &layout, &reference, cli.keep, &cli.shared)
}

fn validate_cli(cli: &Cli) -> Result<()> {
    if !cli.revert && cli.branch.is_none() && cli.tag.is_none() {
        bail!("You must provide one action: --branch, --tag, or --revert");
    }
    if cli.keep == 0 {
        bail!("--keep must be >= 1");
    }
    Ok(())
}

fn build_context(deploy_path: &Path) -> Result<DeployContext> {
    let repo_root = git_toplevel()?;
    let deploy_base = absolute_from_cwd(deploy_path)?;

    Ok(DeployContext {
        repo_root,
        deploy_base,
    })
}

fn deploy(
    ctx: &DeployContext,
    layout: &DeployLayout,
    reference: &RefTarget,
    keep: usize,
    shared_items: &[String],
) -> Result<()> {
    // 1) Sync remote references
    run_git(["fetch", "--all", "--prune"], "Failed to run git fetch")?;

    // 2) Move local checkout to requested target
    checkout_target(reference)?;

    // 3) Create timestamped release directory
    let release_name = make_release_timestamp();
    let release_path = layout.releases.join(&release_name);
    fs::create_dir_all(&release_path).with_context(|| {
        format!(
            "Failed to create release directory {}",
            release_path.display()
        )
    })?;

    // 4) Copy repository content into the new release.
    // IMPORTANT: use repo root as source, and exclude deploy base if it is inside repo.
    copy_repo_to_release(&ctx.repo_root, &release_path, &ctx.deploy_base)?;

    // 5) Switch current symlink to new release
    switch_current_symlink(&layout.current, &release_path)?;

    // 6) Apply shared links inside current release
    apply_shared_links(layout, shared_items)?;

    // 7) Remove old releases
    cleanup_old_releases(&layout.releases, keep)?;

    println!("Deploy completed:");
    println!("  target   : {}", reference.describe());
    println!("  release  : {}", release_name);
    println!("  current  : {}", layout.current.display());

    Ok(())
}

fn ensure_git_repo() -> Result<()> {
    let output = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .context("Failed to execute git rev-parse")?;

    if !output.status.success() {
        bail!("This command must be executed inside a git repository");
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout != "true" {
        bail!("Not inside a git working tree");
    }

    Ok(())
}

fn git_toplevel() -> Result<PathBuf> {
    let top = git_stdout(&["rev-parse", "--show-toplevel"])?;
    let path = PathBuf::from(top);

    if !path.is_dir() {
        bail!("Git top-level path is not a directory: {}", path.display());
    }

    fs::canonicalize(&path).with_context(|| format!("Failed to canonicalize {}", path.display()))
}

fn absolute_from_cwd(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }

    let cwd = std::env::current_dir().context("Failed to get current directory")?;
    Ok(cwd.join(path))
}

fn prepare_layout(base: &Path) -> Result<DeployLayout> {
    let base = canonical_or_original(base);
    let releases = base.join("releases");
    let shared = base.join("shared");
    let current = base.join("current");

    fs::create_dir_all(&releases)
        .with_context(|| format!("Failed to create directory {}", releases.display()))?;
    fs::create_dir_all(&shared)
        .with_context(|| format!("Failed to create directory {}", shared.display()))?;

    Ok(DeployLayout {
        releases,
        shared,
        current,
    })
}

fn canonical_or_original(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn run_git<const N: usize>(args: [&str; N], err_ctx: &str) -> Result<()> {
    let status = Command::new("git")
        .args(args)
        .status()
        .with_context(|| format!("{err_ctx}: failed to invoke git"))?;

    if !status.success() {
        bail!("{err_ctx}");
    }

    Ok(())
}

fn git_stdout(args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .args(args)
        .output()
        .with_context(|| format!("Failed to execute git {:?}", args))?;

    if !out.status.success() {
        bail!("git {:?} returned a non-zero status", args);
    }

    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn checkout_target(target: &RefTarget) -> Result<()> {
    match target {
        RefTarget::Branch(branch) => {
            let remote_ref = format!("refs/remotes/origin/{branch}");
            let exists_remote = Command::new("git")
                .args(["show-ref", "--verify", "--quiet", &remote_ref])
                .status()
                .context("Failed to verify remote branch")?;

            if !exists_remote.success() {
                bail!("Remote branch origin/{branch} does not exist");
            }

            let local_ref = format!("refs/heads/{branch}");
            let exists_local = Command::new("git")
                .args(["show-ref", "--verify", "--quiet", &local_ref])
                .status()
                .context("Failed to verify local branch")?;

            if exists_local.success() {
                run_git(
                    ["checkout", branch.as_str()],
                    "Failed to checkout local branch",
                )?;
                let reset_to = format!("origin/{branch}");
                run_git(
                    ["reset", "--hard", reset_to.as_str()],
                    "Failed to sync local branch with remote",
                )?;
            } else {
                let tracking = format!("origin/{branch}");
                run_git(
                    [
                        "checkout",
                        "-B",
                        branch.as_str(),
                        "--track",
                        tracking.as_str(),
                    ],
                    "Failed to create/track local branch from origin",
                )?;
            }
        }
        RefTarget::Tag(tag) => {
            let tag_ref = format!("refs/tags/{tag}");
            let exists_tag = Command::new("git")
                .args(["show-ref", "--verify", "--quiet", &tag_ref])
                .status()
                .context("Failed to verify tag")?;

            if !exists_tag.success() {
                bail!("Tag '{tag}' does not exist in this repository");
            }

            run_git(
                ["checkout", "--detach", tag.as_str()],
                "Failed to checkout tag",
            )?;
        }
    }

    Ok(())
}

fn make_release_timestamp() -> String {
    Utc::now().format("%Y-%m-%dT%H-%M-%SZ").to_string()
}

fn copy_repo_to_release(
    src_repo_root: &Path,
    dst_release: &Path,
    deploy_base: &Path,
) -> Result<()> {
    let src_repo_root = canonical_or_original(src_repo_root);
    let deploy_base_abs = canonical_or_original(deploy_base);

    // If deploy path is inside repo, this prefix allows us to exclude it entirely.
    let deploy_rel_from_repo = deploy_base_abs
        .strip_prefix(&src_repo_root)
        .ok()
        .map(PathBuf::from);

    copy_dir_filtered(&src_repo_root, dst_release, &|rel| {
        let rel_str = rel.to_string_lossy();

        // Exclude git internals
        if rel_str == ".git" || rel_str.starts_with(".git/") {
            return false;
        }

        // Exclude common build artifacts
        if rel_str == "target" || rel_str.starts_with("target/") {
            return false;
        }

        // Exclude deploy root itself if it is within repository root.
        // This fixes recursive copy path growth when --path points inside or near repo content.
        if let Some(prefix) = &deploy_rel_from_repo {
            if rel == prefix || rel.starts_with(prefix) {
                return false;
            }
        }

        true
    })
}

fn copy_dir_filtered<F>(src_root: &Path, dst_root: &Path, include_rel: &F) -> Result<()>
where
    F: Fn(&Path) -> bool,
{
    if !src_root.is_dir() {
        bail!("Source path is not a directory: {}", src_root.display());
    }

    for entry in walkdir::WalkDir::new(src_root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let src_path = entry.path();
        let rel = match src_path.strip_prefix(src_root) {
            Ok(r) => r,
            Err(_) => continue,
        };

        if rel.as_os_str().is_empty() {
            continue;
        }

        if !include_rel(rel) {
            continue;
        }

        let dst_path = dst_root.join(rel);

        if entry.file_type().is_dir() {
            fs::create_dir_all(&dst_path)
                .with_context(|| format!("Failed to create directory {}", dst_path.display()))?;
        } else if entry.file_type().is_symlink() {
            let target = fs::read_link(src_path)
                .with_context(|| format!("Failed to read symlink {}", src_path.display()))?;
            create_symlink(&target, &dst_path)?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = dst_path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("Failed to create directory {}", parent.display()))?;
            }

            fs::copy(src_path, &dst_path).with_context(|| {
                format!(
                    "Failed to copy file {} -> {}",
                    src_path.display(),
                    dst_path.display()
                )
            })?;
        }
    }

    Ok(())
}

fn switch_current_symlink(current_link: &Path, new_release: &Path) -> Result<()> {
    if current_link.exists() || is_symlink(current_link) {
        remove_path_any(current_link).with_context(|| {
            format!(
                "Failed to remove existing current path: {}",
                current_link.display()
            )
        })?;
    }

    create_symlink(new_release, current_link).with_context(|| {
        format!(
            "Failed to create symlink current {} -> {}",
            current_link.display(),
            new_release.display()
        )
    })?;

    Ok(())
}

fn apply_shared_links(layout: &DeployLayout, shared_items: &[String]) -> Result<()> {
    if shared_items.is_empty() {
        return Ok(());
    }

    let current_resolved = resolve_current_target(&layout.current)?;
    let mut seen = BTreeSet::new();

    for item in shared_items {
        let clean = normalize_relative_path(item)?;
        if !seen.insert(clean.clone()) {
            continue;
        }

        let shared_target = layout.shared.join(&clean);
        if let Some(parent) = shared_target.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "Failed to create parent directory for shared path {}",
                    parent.display()
                )
            })?;
        }

        if !shared_target.exists() {
            fs::create_dir_all(&shared_target).with_context(|| {
                format!(
                    "Failed to create shared directory {}",
                    shared_target.display()
                )
            })?;
        }

        let current_item = current_resolved.join(&clean);

        if current_item.exists() || is_symlink(&current_item) {
            remove_path_any(&current_item).with_context(|| {
                format!(
                    "Failed to remove existing path in current {}",
                    current_item.display()
                )
            })?;
        } else if let Some(parent) = current_item.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "Failed to create parent path in current for {}",
                    current_item.display()
                )
            })?;
        }

        create_symlink(&shared_target, &current_item).with_context(|| {
            format!(
                "Failed to create shared symlink {} -> {}",
                current_item.display(),
                shared_target.display()
            )
        })?;
    }

    Ok(())
}

fn cleanup_old_releases(releases_dir: &Path, keep: usize) -> Result<()> {
    let mut dirs = list_release_dirs_sorted(releases_dir)?;
    if dirs.len() <= keep {
        return Ok(());
    }

    let to_remove = dirs.len() - keep;
    for old in dirs.drain(0..to_remove) {
        fs::remove_dir_all(&old)
            .with_context(|| format!("Failed to remove old release {}", old.display()))?;
    }

    Ok(())
}

fn do_revert(layout: &DeployLayout) -> Result<()> {
    let releases = list_release_dirs_sorted(&layout.releases)?;
    if releases.len() < 2 {
        bail!("Not enough releases to revert (at least 2 required)");
    }

    let previous = releases
        .get(releases.len() - 2)
        .ok_or_else(|| anyhow!("Failed to determine previous release"))?;

    switch_current_symlink(&layout.current, previous)
}

fn list_release_dirs_sorted(releases_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut entries = Vec::new();

    if !releases_dir.exists() {
        return Ok(entries);
    }

    for ent in fs::read_dir(releases_dir)
        .with_context(|| format!("Failed to list {}", releases_dir.display()))?
    {
        let ent = ent?;
        let path = ent.path();
        if path.is_dir() {
            entries.push(path);
        }
    }

    // Lexicographical sort works with YYYY-MM-DDTHH-MM-SSZ
    entries.sort_by(|a, b| {
        let an = a.file_name().and_then(OsStr::to_str).unwrap_or_default();
        let bn = b.file_name().and_then(OsStr::to_str).unwrap_or_default();
        an.cmp(bn)
    });

    Ok(entries)
}

fn resolve_current_target(current_link: &Path) -> Result<PathBuf> {
    if !is_symlink(current_link) {
        bail!(
            "'current' is not a valid symlink: {}",
            current_link.display()
        );
    }

    let target = fs::read_link(current_link)
        .with_context(|| format!("Failed to read symlink {}", current_link.display()))?;

    let resolved = if target.is_absolute() {
        target
    } else {
        current_link
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(target)
    };

    Ok(resolved)
}

fn normalize_relative_path(input: &str) -> Result<PathBuf> {
    let p = Path::new(input);

    if p.is_absolute() {
        bail!("--shared path must be relative: {}", input);
    }

    let mut out = PathBuf::new();
    for c in p.components() {
        use std::path::Component;
        match c {
            Component::CurDir => {}
            Component::Normal(seg) => out.push(seg),
            Component::ParentDir => bail!("'..' is not allowed in --shared: {}", input),
            Component::RootDir | Component::Prefix(_) => {
                bail!("Invalid --shared path: {}", input)
            }
        }
    }

    if out.as_os_str().is_empty() {
        bail!("Empty/invalid --shared path: {}", input);
    }

    Ok(out)
}

fn is_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

fn remove_path_any(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path)
        .with_context(|| format!("Failed to get metadata for {}", path.display()))?;

    let fty = meta.file_type();

    if fty.is_symlink() || fty.is_file() {
        fs::remove_file(path)
            .with_context(|| format!("Failed to remove file/symlink {}", path.display()))?;
    } else if fty.is_dir() {
        fs::remove_dir_all(path)
            .with_context(|| format!("Failed to remove directory {}", path.display()))?;
    } else {
        bail!("Unsupported file type: {}", path.display());
    }

    Ok(())
}

#[cfg(unix)]
fn create_symlink(src: &Path, dst: &Path) -> Result<()> {
    use std::os::unix::fs::symlink;

    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create parent directory {}", parent.display()))?;
    }

    symlink(src, dst).with_context(|| {
        format!(
            "Failed to create symlink {} -> {}",
            dst.display(),
            src.display()
        )
    })?;

    Ok(())
}

#[cfg(windows)]
fn create_symlink(src: &Path, dst: &Path) -> Result<()> {
    use std::os::windows::fs::{symlink_dir, symlink_file};

    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create parent directory {}", parent.display()))?;
    }

    if src.is_dir() {
        symlink_dir(src, dst).with_context(|| {
            format!(
                "Failed to create directory symlink {} -> {}",
                dst.display(),
                src.display()
            )
        })?;
    } else {
        symlink_file(src, dst).with_context(|| {
            format!(
                "Failed to create file symlink {} -> {}",
                dst.display(),
                src.display()
            )
        })?;
    }

    Ok(())
}
