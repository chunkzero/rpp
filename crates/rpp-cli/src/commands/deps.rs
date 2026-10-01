//! `rpp add`, `rpp remove`, `rpp update` and `rpp search`: manage the `dependencies`
//! of `rpp.json` and keep `rpp.lock` in step with them.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use rpp_fetch::registry::{
    parse_dependencies, read_package_summary, resolve, select, validate_name, Dependency,
    DependencySpec, PackageLock, Registry, Update, PACKAGE_MANIFEST,
};
use semver::{Version, VersionReq};

use crate::ordered_json::{Json, Object};
use crate::project::{CONFIG_FILE, LOCK_FILE};
use crate::{atomic, ui};

/// A located `rpp.json` project and its parsed manifest.
struct Manifest {
    root: PathBuf,
    doc: Object,
}

impl Manifest {
    /// Find the nearest ancestor of `dir` holding a project file. A missing `rpp.json`
    /// is an empty manifest when `create` is set (rooted at `dir`), else an error.
    fn discover(dir: &Path, command: &str, create: bool) -> Result<Self> {
        let start = std::path::absolute(dir).context("reading current directory")?;
        let found = start
            .ancestors()
            .find(|d| d.join(PACKAGE_MANIFEST).is_file() || d.join(CONFIG_FILE).is_file());
        let root = match found {
            Some(root) => root.to_path_buf(),
            None if create => start,
            None => bail!(
                "no `{PACKAGE_MANIFEST}` found in `{}` or any parent directory",
                dir.display()
            ),
        };
        let path = root.join(PACKAGE_MANIFEST);
        if !path.is_file() && root.join(CONFIG_FILE).is_file() {
            bail!("this project is configured by {CONFIG_FILE}; use `rpp plugin {command}` there");
        }
        let doc = if path.is_file() {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            match serde_json::from_str(&text)
                .with_context(|| format!("parsing {}", path.display()))?
            {
                Json::Object(doc) => doc,
                _ => bail!("{} must contain a JSON object", path.display()),
            }
        } else {
            Object::default()
        };
        Ok(Self { root, doc })
    }

    fn dependencies_mut(&mut self) -> Result<&mut Object> {
        match self
            .doc
            .entry_or_insert_with("dependencies", || Json::Object(Object::default()))
        {
            Json::Object(deps) => Ok(deps),
            _ => Err(anyhow!(
                "`dependencies` in {PACKAGE_MANIFEST} must be an object"
            )),
        }
    }

    fn dependencies(&self) -> Result<Vec<Dependency>> {
        let text = serde_json::to_string(&self.doc)?;
        Ok(parse_dependencies(&text)?)
    }

    fn lock_path(&self) -> PathBuf {
        self.root.join(LOCK_FILE)
    }

    fn save(&self, lock: &PackageLock) -> Result<()> {
        lock.save(&self.lock_path())
            .with_context(|| format!("writing {}", self.lock_path().display()))?;
        let mut text = serde_json::to_string_pretty(&self.doc)?;
        text.push('\n');
        atomic::write(&self.root.join(PACKAGE_MANIFEST), text)
    }
}

fn rpp_version() -> Result<Version> {
    Version::parse(env!("CARGO_PKG_VERSION")).context("parsing the rpp version")
}

/// Resolve the manifest's dependencies against `lock` in place.
fn resolve_all(manifest: &Manifest, lock: &mut PackageLock, update: &Update) -> Result<()> {
    let registry = Registry::from_env()?;
    resolve(
        &registry,
        &manifest.root,
        &manifest.dependencies()?,
        lock,
        &rpp_version()?,
        update,
    )?;
    Ok(())
}

/// Run `rpp add`.
pub fn add(dir: &Path, specs: &[String]) -> Result<()> {
    ui::intro("Add dependencies");
    let mut manifest = Manifest::discover(dir, "add", true)?;
    let mut added = Vec::new();
    for spec in specs {
        let (name, value) = parse_add_spec(dir, &manifest.root, spec)?;
        added.push(format!("Added {name} {value}"));
        manifest
            .dependencies_mut()?
            .insert(name, Json::String(value));
    }

    let lock_path = manifest.lock_path();
    let mut lock = PackageLock::load(&lock_path)?;
    resolve_all(&manifest, &mut lock, &Update::None)?;
    manifest.save(&lock)?;

    for line in added {
        ui::detail(line);
    }
    ui::success(format!("Updated {PACKAGE_MANIFEST}"));
    Ok(())
}

/// Turn one `add` argument into a dependency name and the spec to store.
fn parse_add_spec(dir: &Path, root: &Path, spec: &str) -> Result<(String, String)> {
    if let Some(path) = spec.strip_prefix("path:") {
        let target = dir.join(path);
        let target = target
            .canonicalize()
            .with_context(|| format!("reading {}", target.display()))?;
        let name = read_package_summary(&target)?.name;
        let root = root.canonicalize().context("resolving the project root")?;
        return Ok((name, format!("path:{}", stored_path(&root, &target))));
    }
    if let Some((name, range)) = spec.split_once('@') {
        validate_name(name)?;
        DependencySpec::parse(name, range)?;
        return Ok((name.to_string(), range.to_string()));
    }
    validate_name(spec)?;
    let entry = Registry::from_env()?.entry(spec)?;
    let chosen = select(&entry, &VersionReq::STAR, &rpp_version()?)?;
    Ok((spec.to_string(), format!("^{}", chosen.version)))
}

/// `target` relative to `root` when it is inside the project or beside it (a sibling
/// of the project directory), else absolute. Always uses forward slashes.
fn stored_path(root: &Path, target: &Path) -> String {
    let relative = target
        .strip_prefix(root)
        .map(Path::to_path_buf)
        .or_else(|_| {
            let parent = root.parent().ok_or(())?;
            let beside = target.strip_prefix(parent).map_err(|_| ())?;
            Ok::<_, ()>(Path::new("..").join(beside))
        })
        .unwrap_or_else(|()| target.to_path_buf());
    if relative.is_absolute() {
        return relative.to_string_lossy().replace('\\', "/");
    }
    let parts: Vec<_> = relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect();
    if parts.is_empty() {
        ".".to_string()
    } else {
        parts.join("/")
    }
}

/// Run `rpp remove`.
pub fn remove(dir: &Path, names: &[String]) -> Result<()> {
    ui::intro("Remove dependencies");
    let mut manifest = Manifest::discover(dir, "remove", false)?;
    for name in names {
        if manifest.dependencies_mut()?.remove(name).is_none() {
            bail!("`{name}` is not a dependency in {PACKAGE_MANIFEST}");
        }
    }
    let remaining = manifest.dependencies()?;
    let kept: Vec<&str> = remaining.iter().map(|d| d.name.as_str()).collect();
    let mut lock = PackageLock::load(&manifest.lock_path())?;
    lock.retain_names(&kept);
    manifest.save(&lock)?;

    for name in names {
        ui::detail(format!("Removed {name}"));
    }
    ui::success(format!("Updated {PACKAGE_MANIFEST}"));
    Ok(())
}

/// Run `rpp update`.
pub fn update(dir: &Path, names: &[String]) -> Result<()> {
    ui::intro("Update dependencies");
    let manifest = Manifest::discover(dir, "update", false)?;
    let registry_names: Vec<String> = manifest
        .dependencies()?
        .into_iter()
        .filter(|d| matches!(d.spec, DependencySpec::Registry(_)))
        .map(|d| d.name)
        .collect();
    for name in names {
        if !registry_names.contains(name) {
            bail!("`{name}` is not a registry dependency in {PACKAGE_MANIFEST}");
        }
    }

    let (targets, mode) = if names.is_empty() {
        (registry_names, Update::All)
    } else {
        (names.to_vec(), Update::Only(names.to_vec()))
    };
    let mut lock = PackageLock::load(&manifest.lock_path())?;
    let before: Vec<Option<Version>> = targets
        .iter()
        .map(|n| lock.get(n).map(|p| p.version.clone()))
        .collect();
    resolve_all(&manifest, &mut lock, &mode)?;
    manifest.save(&lock)?;

    for (name, old) in targets.iter().zip(before) {
        let new = lock.get(name).map(|p| &p.version);
        match (old, new) {
            (Some(old), Some(new)) if &old != new => ui::detail(format!("{name} {old} → {new}")),
            _ => ui::detail(format!("{name} up to date")),
        }
    }
    ui::success(format!("Updated {LOCK_FILE}"));
    Ok(())
}

/// Run `rpp search`.
pub fn search(query: &str) -> Result<()> {
    let hits = Registry::from_env()?
        .search(query)
        .context("searching the registry")?;
    if hits.is_empty() {
        ui::detail("no matching plugins");
        return Ok(());
    }
    for hit in hits {
        println!(
            "{} {}  {}",
            hit.name,
            hit.latest,
            hit.description.unwrap_or_default()
        );
    }
    Ok(())
}
