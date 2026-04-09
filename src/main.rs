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
    about = "Git plugin para despliegues estilo Capistrano",
    long_about = "Plugin de Git para desplegar rama/tag del repo actual a una estructura tipo Capistrano:\n\
                  - releases/<timestamp>\n\
                  - current (symlink)\n\
                  - shared\n\
                  Soporta rollback con --revert y rutas compartidas con --shared."
)]
struct Cli {
    /// Rama a desplegar (mutuamente excluyente con --tag y --revert)
    #[arg(short = 'b', long = "branch", conflicts_with_all = ["tag", "revert"])]
    branch: Option<String>,

    /// Tag a desplegar (mutuamente excluyente con --branch y --revert)
    #[arg(short = 't', long = "tag", conflicts_with_all = ["branch", "revert"])]
    tag: Option<String>,

    /// Revertir current a la release anterior (mutuamente excluyente con --branch/--tag)
    #[arg(short = 'r', long = "revert", action = ArgAction::SetTrue, conflicts_with_all = ["branch", "tag"])]
    revert: bool,

    /// Ruta base de despliegue (ej: /www/folder)
    #[arg(short = 'p', long = "path")]
    deploy_path: PathBuf,

    /// Número de releases a mantener (default: 3)
    #[arg(short = 'k', long = "keep", default_value_t = 3)]
    keep: usize,

    /// Rutas compartidas (repetible), ej:
    /// --shared node_modules --shared vendor/subfolder
    #[arg(long = "shared")]
    shared: Vec<String>,
}

#[derive(Debug)]
struct DeployLayout {
    base: PathBuf,
    releases: PathBuf,
    shared: PathBuf,
    current: PathBuf,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if !cli.revert && cli.branch.is_none() && cli.tag.is_none() {
        bail!("Debes indicar una acción: --branch, --tag o --revert");
    }

    ensure_git_repo()?;

    let layout = prepare_layout(&cli.deploy_path)?;

    if cli.revert {
        do_revert(&layout)?;
        println!("Revert completado. 'current' apunta a la release anterior.");
        return Ok(());
    }

    let reference = match (&cli.branch, &cli.tag) {
        (Some(branch), None) => RefTarget::Branch(branch.clone()),
        (None, Some(tag)) => RefTarget::Tag(tag.clone()),
        _ => bail!("Debes usar exactamente una de estas opciones: --branch o --tag"),
    };

    // 1) Sincronizar remoto
    run_git(
        ["fetch", "--all", "--prune"],
        "No se pudo ejecutar git fetch",
    )?;

    // 2) Actualizar checkout local al target solicitado
    checkout_target(&reference)?;

    // 3) Crear release timestamp
    let release_name = make_release_timestamp();
    let release_path = layout.releases.join(&release_name);
    fs::create_dir_all(&release_path)
        .with_context(|| format!("No se pudo crear release {:?}", release_path))?;

    // 4) Copiar working tree actual a release (excluyendo .git y artefactos comunes)
    copy_repo_to_release(Path::new("."), &release_path)?;

    // 5) Actualizar symlink current -> nueva release
    switch_current_symlink(&layout.current, &release_path)?;

    // 6) Aplicar shared links sobre current
    apply_shared_links(&layout, &cli.shared)?;

    // 7) Limpiar releases viejas
    cleanup_old_releases(&layout.releases, cli.keep)?;

    println!("Deploy completado:");
    println!("  target   : {}", reference.describe());
    println!("  release  : {}", release_name);
    println!("  current  : {}", layout.current.display());

    Ok(())
}

#[derive(Debug)]
enum RefTarget {
    Branch(String),
    Tag(String),
}

impl RefTarget {
    fn describe(&self) -> String {
        match self {
            RefTarget::Branch(b) => format!("branch:{}", b),
            RefTarget::Tag(t) => format!("tag:{}", t),
        }
    }
}

fn ensure_git_repo() -> Result<()> {
    let output = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .context("No se pudo ejecutar git rev-parse")?;

    if !output.status.success() {
        bail!("Este comando debe ejecutarse dentro de un repositorio git");
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout != "true" {
        bail!("No estás dentro de un working tree git");
    }
    Ok(())
}

fn prepare_layout(base: &Path) -> Result<DeployLayout> {
    let base = canonical_or_original(base);
    let releases = base.join("releases");
    let shared = base.join("shared");
    let current = base.join("current");

    fs::create_dir_all(&releases)
        .with_context(|| format!("No se pudo crear directorio {:?}", releases))?;
    fs::create_dir_all(&shared)
        .with_context(|| format!("No se pudo crear directorio {:?}", shared))?;

    Ok(DeployLayout {
        base,
        releases,
        shared,
        current,
    })
}

fn canonical_or_original(path: &Path) -> PathBuf {
    // Si no existe todavía, devolvemos el path original.
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn run_git<const N: usize>(args: [&str; N], err_ctx: &str) -> Result<()> {
    let status = Command::new("git")
        .args(args)
        .status()
        .with_context(|| format!("{}: fallo al invocar git", err_ctx))?;

    if !status.success() {
        bail!("{}", err_ctx);
    }
    Ok(())
}

fn git_stdout(args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .args(args)
        .output()
        .with_context(|| format!("No se pudo ejecutar git {:?}", args))?;
    if !out.status.success() {
        bail!("git {:?} devolvió error", args);
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn checkout_target(target: &RefTarget) -> Result<()> {
    match target {
        RefTarget::Branch(branch) => {
            // Confirmar que existe en remoto
            let remote_ref = format!("refs/remotes/origin/{}", branch);
            let exists_remote = Command::new("git")
                .args(["show-ref", "--verify", "--quiet", &remote_ref])
                .status()
                .context("No se pudo verificar rama remota")?;

            if !exists_remote.success() {
                bail!("La rama remota origin/{} no existe", branch);
            }

            // Si la rama local existe: reset a origin/branch
            let local_ref = format!("refs/heads/{}", branch);
            let exists_local = Command::new("git")
                .args(["show-ref", "--verify", "--quiet", &local_ref])
                .status()
                .context("No se pudo verificar rama local")?;

            if exists_local.success() {
                run_git(
                    ["checkout", branch.as_str()],
                    "No se pudo hacer checkout de la rama local",
                )?;
                let reset_to = format!("origin/{}", branch);
                run_git(
                    ["reset", "--hard", reset_to.as_str()],
                    "No se pudo sincronizar la rama local con remoto",
                )?;
            } else {
                run_git(
                    [
                        "checkout",
                        "-B",
                        branch.as_str(),
                        "--track",
                        &format!("origin/{}", branch),
                    ],
                    "No se pudo crear/trakear la rama local desde origin",
                )?;
            }
        }
        RefTarget::Tag(tag) => {
            // Verificar que tag existe
            let tag_ref = format!("refs/tags/{}", tag);
            let exists_tag = Command::new("git")
                .args(["show-ref", "--verify", "--quiet", &tag_ref])
                .status()
                .context("No se pudo verificar tag")?;

            if !exists_tag.success() {
                bail!("La tag '{}' no existe en este repositorio", tag);
            }

            // Checkout detached al tag
            run_git(
                ["checkout", "--detach", tag.as_str()],
                "No se pudo hacer checkout de la tag",
            )?;
        }
    }

    Ok(())
}

fn make_release_timestamp() -> String {
    // Formato seguro para nombre de carpeta
    // Ejemplo: 2026-01-10T20-31-05Z
    Utc::now().format("%Y-%m-%dT%H-%M-%SZ").to_string()
}

fn copy_repo_to_release(src_repo_root: &Path, dst_release: &Path) -> Result<()> {
    // Copia recursiva simple filtrando entradas.
    // Excluye .git y directorios de build comunes.
    copy_dir_filtered(src_repo_root, dst_release, &|rel| {
        let rel_str = rel.to_string_lossy();

        if rel_str == ".git" || rel_str.starts_with(".git/") {
            return false;
        }

        // Evitar copiar el propio directorio de despliegue si está dentro del repo
        if rel_str.starts_with("releases/")
            || rel_str == "releases"
            || rel_str.starts_with("shared/")
            || rel_str == "shared"
            || rel_str == "current"
        {
            return false;
        }

        // Excluir algunos artefactos típicos
        if rel_str.starts_with("target/") || rel_str == "target" {
            return false;
        }

        true
    })
}

fn copy_dir_filtered<F>(src_root: &Path, dst_root: &Path, include_rel: &F) -> Result<()>
where
    F: Fn(&Path) -> bool,
{
    if !src_root.is_dir() {
        bail!("Ruta origen no es un directorio: {}", src_root.display());
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
            if entry.file_type().is_dir() {
                // walkdir no tiene prune nativo aquí; el filtro include se aplica por cada item.
            }
            continue;
        }

        let dst_path = dst_root.join(rel);

        if entry.file_type().is_dir() {
            fs::create_dir_all(&dst_path)
                .with_context(|| format!("No se pudo crear directorio {}", dst_path.display()))?;
        } else if entry.file_type().is_symlink() {
            // Copiamos symlink como symlink
            let target = fs::read_link(src_path)
                .with_context(|| format!("No se pudo leer symlink {}", src_path.display()))?;
            create_symlink(&target, &dst_path)?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = dst_path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("No se pudo crear directorio {}", parent.display()))?;
            }
            fs::copy(src_path, &dst_path).with_context(|| {
                format!(
                    "No se pudo copiar archivo {} -> {}",
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
                "No se pudo eliminar current existente: {}",
                current_link.display()
            )
        })?;
    }

    create_symlink(new_release, current_link).with_context(|| {
        format!(
            "No se pudo crear symlink current {} -> {}",
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
            continue; // evitar duplicados
        }

        let shared_target = layout.shared.join(&clean);
        if let Some(parent) = shared_target.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "No se pudo crear directorio padre de shared {}",
                    parent.display()
                )
            })?;
        }

        // Asegurar que existe el destino en shared como directorio
        if !shared_target.exists() {
            fs::create_dir_all(&shared_target).with_context(|| {
                format!(
                    "No se pudo crear directorio shared {}",
                    shared_target.display()
                )
            })?;
        }

        let current_item = current_resolved.join(&clean);

        // Si existe en current, borrarlo (archivo, dir o symlink)
        if current_item.exists() || is_symlink(&current_item) {
            remove_path_any(&current_item).with_context(|| {
                format!(
                    "No se pudo eliminar ruta existente en current {}",
                    current_item.display()
                )
            })?;
        } else if let Some(parent) = current_item.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "No se pudo crear padre en current para {}",
                    current_item.display()
                )
            })?;
        }

        create_symlink(&shared_target, &current_item).with_context(|| {
            format!(
                "No se pudo crear symlink shared {} -> {}",
                current_item.display(),
                shared_target.display()
            )
        })?;
    }

    Ok(())
}

fn cleanup_old_releases(releases_dir: &Path, keep: usize) -> Result<()> {
    if keep == 0 {
        bail!("--keep debe ser >= 1");
    }

    let mut dirs = list_release_dirs_sorted(releases_dir)?;
    if dirs.len() <= keep {
        return Ok(());
    }

    let to_remove = dirs.len() - keep;
    for old in dirs.drain(0..to_remove) {
        fs::remove_dir_all(&old)
            .with_context(|| format!("No se pudo eliminar release vieja {}", old.display()))?;
    }

    Ok(())
}

fn do_revert(layout: &DeployLayout) -> Result<()> {
    let releases = list_release_dirs_sorted(&layout.releases)?;
    if releases.len() < 2 {
        bail!("No hay suficientes releases para revertir (se requieren al menos 2)");
    }

    // La penúltima release pasa a ser current
    let previous = releases
        .get(releases.len() - 2)
        .ok_or_else(|| anyhow!("No se pudo determinar la release anterior"))?;

    switch_current_symlink(&layout.current, previous)?;
    Ok(())
}

fn list_release_dirs_sorted(releases_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut entries = Vec::new();

    if !releases_dir.exists() {
        return Ok(entries);
    }

    for ent in fs::read_dir(releases_dir)
        .with_context(|| format!("No se pudo listar {}", releases_dir.display()))?
    {
        let ent = ent?;
        let path = ent.path();
        if path.is_dir() {
            entries.push(path);
        }
    }

    // Orden lexicográfico ascendente; funciona con formato timestamp YYYY-MM-DDTHH-MM-SSZ
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
            "'current' no es un symlink válido: {}",
            current_link.display()
        );
    }

    let target = fs::read_link(current_link)
        .with_context(|| format!("No se pudo leer symlink {}", current_link.display()))?;

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
        bail!("La ruta --shared debe ser relativa: {}", input);
    }

    let mut out = PathBuf::new();
    for c in p.components() {
        use std::path::Component;
        match c {
            Component::CurDir => {}
            Component::Normal(seg) => out.push(seg),
            Component::ParentDir => bail!("No se permite '..' en --shared: {}", input),
            Component::RootDir | Component::Prefix(_) => {
                bail!("Ruta --shared inválida: {}", input)
            }
        }
    }

    if out.as_os_str().is_empty() {
        bail!("Ruta --shared vacía/inválida: {}", input);
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
        .with_context(|| format!("No se pudo obtener metadata de {}", path.display()))?;

    let fty = meta.file_type();

    if fty.is_symlink() || fty.is_file() {
        fs::remove_file(path)
            .with_context(|| format!("No se pudo eliminar archivo/symlink {}", path.display()))?;
    } else if fty.is_dir() {
        fs::remove_dir_all(path)
            .with_context(|| format!("No se pudo eliminar directorio {}", path.display()))?;
    } else {
        bail!("Tipo de archivo no soportado: {}", path.display());
    }

    Ok(())
}

#[cfg(unix)]
fn create_symlink(src: &Path, dst: &Path) -> Result<()> {
    use std::os::unix::fs::symlink;
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("No se pudo crear directorio padre {}", parent.display()))?;
    }
    symlink(src, dst).with_context(|| {
        format!(
            "No se pudo crear symlink {} -> {}",
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
            .with_context(|| format!("No se pudo crear directorio padre {}", parent.display()))?;
    }

    if src.is_dir() {
        symlink_dir(src, dst).with_context(|| {
            format!(
                "No se pudo crear symlink de directorio {} -> {}",
                dst.display(),
                src.display()
            )
        })?;
    } else {
        symlink_file(src, dst).with_context(|| {
            format!(
                "No se pudo crear symlink de archivo {} -> {}",
                dst.display(),
                src.display()
            )
        })?;
    }

    Ok(())
}
