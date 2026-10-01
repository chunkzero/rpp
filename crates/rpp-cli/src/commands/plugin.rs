//! `rpp plugin ...`: manage project and user-level plugin manifests (via
//! `toml_edit`, preserving formatting and comments) and their lockfile pins.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use clap::Subcommand;
use rpp::config::PluginConfig;
use rpp_fetch::{search, Lockfile, PluginSource, Resolver};

use crate::atomic;
use crate::project::{resolve_plugin_meta, validate_plugin_dir, Project};
use crate::ui;
use crate::user_plugins::{copy_plugin_dir, UserPlugins};

mod edit;
mod pack;

pub use edit::{add_plugin, PluginEntry};
pub use pack::Packed;

/// Plugin subcommands (`add`/`remove`/`list`/`update`/`search`/`pack`).
#[derive(Debug, Subcommand)]
pub enum PluginCommand {
    /// Add a plugin and resolve it immediately.
    Add {
        /// Source string, GitHub source, or plugin package directory.
        source: String,
        /// Git ref (tag/branch/sha) for GitHub sources.
        #[arg(long = "ref")]
        r#ref: Option<String>,
        /// Optional subdir within a GitHub repo.
        #[arg(long)]
        subdir: Option<String>,
        /// Install for every project in the user plugin directory.
        #[arg(long, conflicts_with = "project")]
        global: bool,
        /// Install only in the current project's manifest.
        #[arg(long, conflicts_with = "global")]
        project: bool,
    },
    /// Remove a plugin by id or source string.
    Remove {
        /// The plugin id or source string to remove.
        id: String,
        /// Remove from the user-level plugin manifest.
        #[arg(long)]
        global: bool,
    },
    /// List configured plugins.
    List {
        /// List only user-level plugins.
        #[arg(long)]
        global: bool,
    },
    /// Re-resolve plugins (ignoring existing pins), updating the lockfile.
    Update {
        /// An optional single plugin (id or source) to update.
        id: Option<String>,
        /// Update user-level plugins.
        #[arg(long)]
        global: bool,
    },
    /// Search GitHub for `rpp-plugin`-topic repositories.
    Search {
        /// The search query.
        query: String,
    },
    /// Bundle a plugin into a self-contained `<name>-<version>.rpp.tgz` archive.
    Pack {
        /// The plugin directory (default: the current directory).
        dir: Option<PathBuf>,
        /// Directory for the archive and its `.sha256` file (default: the plugin directory).
        #[arg(long)]
        out: Option<PathBuf>,
        /// Print the result as JSON on stdout.
        #[arg(long)]
        json: bool,
    },
}

/// Run a plugin subcommand from the current directory.
pub fn run(dir: &Path, command: PluginCommand) -> Result<()> {
    match &command {
        PluginCommand::Add { .. } => ui::intro("Add a plugin"),
        PluginCommand::Remove { .. } => ui::intro("Remove a plugin"),
        PluginCommand::Update { .. } => ui::intro("Update plugins"),
        PluginCommand::List { .. } | PluginCommand::Search { .. } | PluginCommand::Pack { .. } => {}
    }
    match command {
        PluginCommand::Add {
            source,
            r#ref,
            subdir,
            global,
            project,
        } => add(
            dir,
            &source,
            r#ref.as_deref(),
            subdir.as_deref(),
            install_scope(global, project)?,
        ),
        PluginCommand::Remove { id, global } => remove(dir, &id, global),
        PluginCommand::List { global } => list(dir, global),
        PluginCommand::Update { id, global } => update(dir, id.as_deref(), global),
        PluginCommand::Search { query } => run_search(&query),
        PluginCommand::Pack {
            dir: plugin_dir,
            out,
            json,
        } => run_pack(dir, plugin_dir.as_deref(), out.as_deref(), json),
    }
}

fn run_pack(dir: &Path, plugin: Option<&Path>, out: Option<&Path>, json: bool) -> Result<()> {
    let plugin = plugin.map_or_else(|| dir.to_path_buf(), |p| dir.join(p));
    let out = out.map_or_else(|| plugin.clone(), |o| dir.join(o));
    let packed = pack::pack(&plugin, &out)?;
    if json {
        println!("{}", serde_json::to_string(&packed)?);
    } else {
        ui::success(format!(
            "Packed {} {} into {} (sha256 {})",
            packed.name,
            packed.version,
            packed.path.display(),
            packed.sha256
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InstallScope {
    Project,
    Global,
}

fn install_scope(global: bool, project: bool) -> Result<InstallScope> {
    if global {
        return Ok(InstallScope::Global);
    }
    if project || !ui::is_interactive() {
        return Ok(InstallScope::Project);
    }
    Ok(cliclack::select("Where should this plugin be installed?")
        .items(&[
            (InstallScope::Project, "This project", ""),
            (InstallScope::Global, "Globally for this user", ""),
        ])
        .interact()?)
}

fn add(
    dir: &Path,
    source: &str,
    ref_: Option<&str>,
    subdir: Option<&str>,
    scope: InstallScope,
) -> Result<()> {
    match scope {
        InstallScope::Project => add_project(dir, source, ref_, subdir),
        InstallScope::Global => add_global(dir, source, ref_, subdir),
    }
}

fn add_project(dir: &Path, source: &str, ref_: Option<&str>, subdir: Option<&str>) -> Result<()> {
    let project = Project::discover(dir)?;
    let source = normalize_source(source, dir)?;
    let parsed = PluginSource::parse(&source, ref_, subdir)
        .with_context(|| format!("invalid plugin source `{source}`"))?;

    let resolver = Resolver::new(&project.root).context("initializing resolver")?;
    let lock_path = project.lock_path();
    let mut lock = Lockfile::load(&lock_path)?;

    let resolved = resolver
        .resolve(
            &parsed,
            lock.get_for(&parsed.canonical(), parsed.requested_ref(), parsed.subdir()),
        )
        .with_context(|| format!("resolving plugin `{source}`"))?;

    let (id, version) = validate_plugin_dir(&resolved.root)?;

    let config_path = project.config_path();
    let text = std::fs::read_to_string(&config_path)
        .with_context(|| format!("reading {}", config_path.display()))?;
    let updated = add_plugin(
        &text,
        PluginEntry {
            source: source.clone(),
            r#ref: ref_.map(str::to_string),
            subdir: subdir.map(str::to_string),
            origin: None,
        },
    )?;

    // Write lockfile before rpp.toml so a failed config write leaves a pin
    // without a config entry (recoverable via remove) rather than the reverse.
    if resolved.pinned.is_some() {
        lock.record_resolved(&parsed, &resolved);
        lock.save(&lock_path)?;
    }
    atomic::write(&config_path, &updated)?;

    ui::success(format!("Added project plugin `{id}` v{version}"));
    Ok(())
}

fn add_global(dir: &Path, source: &str, ref_: Option<&str>, subdir: Option<&str>) -> Result<()> {
    let user = UserPlugins::load()?;
    let source = normalize_source(source, dir)?;
    let parsed = PluginSource::parse(&source, ref_, subdir)
        .with_context(|| format!("invalid plugin source `{source}`"))?;
    let resolver_root = absolute_dir(dir)?;
    let resolver = Resolver::new(&resolver_root).context("initializing resolver")?;
    let mut lock = user.lockfile()?;
    let resolved = resolver
        .resolve(
            &parsed,
            lock.get_for(&parsed.canonical(), parsed.requested_ref(), parsed.subdir()),
        )
        .with_context(|| format!("resolving plugin `{source}`"))?;
    let (id, version) = validate_plugin_dir(&resolved.root)?;

    let is_directory = matches!(parsed, PluginSource::Path { .. });
    let installed_source = if is_directory {
        format!("path:plugins/{id}")
    } else {
        source
    };
    let text = user.manifest_text()?;
    let updated = add_plugin(
        &text,
        PluginEntry {
            source: installed_source,
            r#ref: ref_.map(str::to_string),
            subdir: subdir.map(str::to_string),
            origin: is_directory.then(|| resolved.root.display().to_string()),
        },
    )?;

    user.ensure_root()?;
    if is_directory {
        copy_plugin_dir(&resolved.root, &user.plugin_dir(&id))?;
    }
    if resolved.pinned.is_some() {
        lock.record_resolved(&parsed, &resolved);
        lock.save(&user.lock_path())?;
    }
    atomic::write(&user.manifest_path(), updated)?;

    ui::success(format!("Installed global plugin `{id}` v{version}"));
    Ok(())
}

fn normalize_source(source: &str, dir: &Path) -> Result<String> {
    let source = source.trim();
    if source.starts_with("path:") || source.starts_with("github:") {
        return Ok(source.to_string());
    }

    let base = absolute_dir(dir)?;
    let candidate = PathBuf::from(source);
    let resolved = if candidate.is_absolute() {
        candidate
    } else {
        base.join(candidate)
    };
    if !resolved.is_dir() {
        anyhow::bail!(
            "plugin source `{source}` is neither `path:...`, `github:owner/repo`, nor a directory"
        );
    }
    let resolved = resolved
        .canonicalize()
        .with_context(|| format!("canonicalizing {}", resolved.display()))?;
    Ok(format!("path:{}", resolved.display()))
}

fn absolute_dir(dir: &Path) -> Result<PathBuf> {
    if dir.is_absolute() {
        Ok(dir.to_path_buf())
    } else {
        Ok(std::env::current_dir()
            .context("reading current directory")?
            .join(dir))
    }
}

fn remove(dir: &Path, id_or_source: &str, global: bool) -> Result<()> {
    if global {
        return remove_global(id_or_source);
    }
    let project = Project::discover(dir)?;
    let config_path = project.config_path();
    let text = std::fs::read_to_string(&config_path)
        .with_context(|| format!("reading {}", config_path.display()))?;
    let lock_path = project.lock_path();
    let mut lock = Lockfile::load(&lock_path)?;
    let resolver = Resolver::new(&project.root).context("initializing resolver")?;

    let index = find_plugin_in(&project.config.plugins, id_or_source, &lock, &resolver)?
        .ok_or_else(|| anyhow!("no plugin matching `{id_or_source}` found"))?;

    let plugin = &project.config.plugins[index];
    let updated = edit::remove_plugin_at(&text, index)?;
    let removed_source = plugin.source.as_deref();
    atomic::write(&config_path, &updated)?;

    if let Some(removed_source) = removed_source {
        if let Ok(parsed) = PluginSource::parse(
            removed_source,
            plugin.r#ref.as_deref(),
            plugin.subdir.as_deref(),
        ) {
            if lock
                .remove_for(&parsed.canonical(), parsed.requested_ref(), parsed.subdir())
                .is_some()
            {
                lock.save(&lock_path)?;
            }
        }
    }

    ui::success(format!("Removed plugin `{id_or_source}`"));
    Ok(())
}

fn remove_global(id_or_source: &str) -> Result<()> {
    let user = UserPlugins::load()?;
    let text = user.manifest_text()?;
    let mut lock = user.lockfile()?;
    let resolver = Resolver::new(&user.root).context("initializing resolver")?;
    let index = find_plugin_in(&user.plugins, id_or_source, &lock, &resolver)?
        .ok_or_else(|| anyhow!("no global plugin matching `{id_or_source}` found"))?;
    let plugin = &user.plugins[index];
    let meta = resolve_plugin_meta(plugin, &lock, &resolver)?;
    let Some(source) = plugin.source.as_deref() else {
        return Err(anyhow!("global plugin `{id_or_source}` has no source"));
    };
    let updated = edit::remove_plugin_at(&text, index)?;
    let removed_source = source;
    atomic::write(&user.manifest_path(), updated)?;

    if let Ok(parsed) = PluginSource::parse(
        removed_source,
        plugin.r#ref.as_deref(),
        plugin.subdir.as_deref(),
    ) {
        if lock
            .remove_for(&parsed.canonical(), parsed.requested_ref(), parsed.subdir())
            .is_some()
        {
            lock.save(&user.lock_path())?;
        }
    }
    if let Some(meta) = meta {
        let installed = user.plugin_dir(&meta.id);
        if removed_source == format!("path:plugins/{}", meta.id) && installed.is_dir() {
            std::fs::remove_dir_all(&installed)
                .with_context(|| format!("removing {}", installed.display()))?;
        }
    }

    ui::success(format!("Removed global plugin `{id_or_source}`"));
    Ok(())
}

fn list(dir: &Path, global_only: bool) -> Result<()> {
    if global_only {
        let user = UserPlugins::load()?;
        return list_plugins(&user.plugins, &user.lock_path(), &user.root, "global");
    }
    let project = Project::discover(dir)?;
    if project.user_plugins.plugins.is_empty() && project.config.plugins.is_empty() {
        ui::detail("no plugins configured");
        return Ok(());
    }
    if !project.user_plugins.plugins.is_empty() {
        list_plugins(
            &project.user_plugins.plugins,
            &project.user_plugins.lock_path(),
            &project.user_plugins.root,
            "global",
        )?;
    }
    if !project.config.plugins.is_empty() {
        list_plugins(
            &project.config.plugins,
            &project.lock_path(),
            &project.root,
            "project",
        )?;
    }
    Ok(())
}

fn list_plugins(
    plugins: &[PluginConfig],
    lock_path: &Path,
    resolver_root: &Path,
    scope: &str,
) -> Result<()> {
    if plugins.is_empty() {
        ui::detail(format!("no {scope} plugins configured"));
        return Ok(());
    }
    let lock = Lockfile::load(lock_path)?;
    let resolver = Resolver::new(resolver_root).context("initializing resolver")?;
    for plugin in plugins {
        let parsed = plugin.source.as_deref().and_then(|source| {
            PluginSource::parse(source, plugin.r#ref.as_deref(), plugin.subdir.as_deref()).ok()
        });
        let pin = parsed
            .as_ref()
            .and_then(|p| lock.get_for(&p.canonical(), p.requested_ref(), p.subdir()))
            .map(|l| format!("{}@{}", l.ref_, short_commit(&l.commit)));

        let id_ver = match resolve_plugin_meta(plugin, &lock, &resolver)? {
            Some(meta) => format!("{} v{}", meta.id, meta.version),
            None => plugin
                .id
                .as_deref()
                .map(|id| format!("{id} (global reference)"))
                .unwrap_or_else(|| "(unresolved)".to_string()),
        };
        let label = plugin.label();
        match pin {
            Some(pin) => println!("  {id_ver}  [{scope}: {label}]  pin={pin}"),
            None => println!("  {id_ver}  [{scope}: {label}]"),
        }
    }
    Ok(())
}

fn update(dir: &Path, id_or_source: Option<&str>, global: bool) -> Result<()> {
    let (changed, matched) = if global {
        let user = UserPlugins::load()?;
        let (mut changed, mut matched) =
            update_plugins(&user.plugins, &user.root, &user.lock_path(), id_or_source)?;
        let (copied, copied_matched) = refresh_copied_plugins(&user, id_or_source)?;
        changed += copied;
        matched |= copied_matched;
        (changed, matched)
    } else {
        let project = Project::discover(dir)?;
        update_plugins(
            &project.config.plugins,
            &project.root,
            &project.lock_path(),
            id_or_source,
        )?
    };

    if let (Some(target), false) = (id_or_source, matched) {
        return Err(anyhow!("no plugin matching `{target}` found"));
    }
    ui::success(format!(
        "{changed} plugin{} updated",
        if changed == 1 { "" } else { "s" }
    ));
    Ok(())
}

/// Re-copy directory-installed global plugins from their recorded `origin`.
/// Returns `(refreshed, matched)`.
fn refresh_copied_plugins(user: &UserPlugins, id_or_source: Option<&str>) -> Result<(usize, bool)> {
    let mut refreshed = 0usize;
    let mut matched = false;
    for plugin in &user.plugins {
        let Some(source) = plugin.source.as_deref() else {
            continue;
        };
        let Some(origin) = user.origins.get(source) else {
            continue;
        };
        let id = source
            .strip_prefix("path:plugins/")
            .ok_or_else(|| anyhow!("global plugin `{source}` has an unexpected install source"))?;
        if id_or_source.is_some_and(|target| target != id && target != source) {
            continue;
        }
        matched = true;
        if !origin.is_dir() {
            ui::warn(format!(
                "global plugin `{id}`: origin {} no longer exists; skipped",
                origin.display()
            ));
            continue;
        }
        let (origin_id, version) = validate_plugin_dir(origin)?;
        if origin_id != id {
            anyhow::bail!(
                "global plugin `{id}`: origin {} now contains plugin `{origin_id}`",
                origin.display()
            );
        }
        copy_plugin_dir(origin, &user.plugin_dir(id))?;
        ui::detail(format!("{id} <- {} (v{version})", origin.display()));
        refreshed += 1;
    }
    Ok((refreshed, matched))
}

/// Re-resolve GitHub plugins and record new pins. Returns `(changed, matched)`.
fn update_plugins(
    plugins: &[PluginConfig],
    resolver_root: &Path,
    lock_path: &Path,
    id_or_source: Option<&str>,
) -> Result<(usize, bool)> {
    let resolver = Resolver::new(resolver_root).context("initializing resolver")?;
    update_plugins_with_resolver(plugins, lock_path, id_or_source, &resolver)
}

fn update_plugins_with_resolver(
    plugins: &[PluginConfig],
    lock_path: &Path,
    id_or_source: Option<&str>,
    resolver: &Resolver,
) -> Result<(usize, bool)> {
    let mut lock = Lockfile::load(lock_path)?;
    let mut changed = 0usize;
    let mut matched = false;

    let selected = id_or_source
        .map(|target| find_plugin_in(plugins, target, &lock, resolver))
        .transpose()?
        .flatten();
    for (index, plugin) in plugins.iter().enumerate() {
        if id_or_source.is_some() && selected != Some(index) {
            continue;
        }
        matched = true;

        let Some(source) = plugin.source.as_deref() else {
            continue;
        };
        let parsed =
            match PluginSource::parse(source, plugin.r#ref.as_deref(), plugin.subdir.as_deref()) {
                Ok(p) => p,
                Err(_) => continue,
            };

        if matches!(parsed, PluginSource::Path { .. }) {
            continue;
        }

        let resolved = resolver
            .resolve(&parsed, None)
            .with_context(|| format!("updating plugin `{source}`"))?;
        if let Some(pin) = &resolved.pinned {
            let prev = lock.record_resolved(&parsed, &resolved);
            if prev.map(|p| p.commit) != Some(pin.commit.clone()) {
                changed += 1;
                ui::detail(format!("{source} -> {}", short_commit(&pin.commit)));
            }
        }
    }

    if changed > 0 {
        lock.save(lock_path)?;
    }
    Ok((changed, matched))
}

fn find_plugin_in(
    plugins: &[PluginConfig],
    id_or_source: &str,
    lock: &Lockfile,
    resolver: &Resolver,
) -> Result<Option<usize>> {
    let mut selected = None;
    for (index, plugin) in plugins.iter().enumerate() {
        let matches = if plugin.source.as_deref() == Some(id_or_source)
            || plugin.id.as_deref() == Some(id_or_source)
        {
            true
        } else {
            resolve_plugin_meta(plugin, lock, resolver)?
                .is_some_and(|meta| meta.id == id_or_source || meta.canonical == id_or_source)
        };
        if matches {
            anyhow::ensure!(
                selected.is_none(),
                "multiple plugins match `{id_or_source}`; select a plugin id"
            );
            selected = Some(index);
        }
    }
    Ok(selected)
}

fn run_search(query: &str) -> Result<()> {
    let hits = search(query).context("searching GitHub")?;
    if hits.is_empty() {
        ui::detail("no matching repositories");
        return Ok(());
    }
    for hit in hits {
        let desc = hit.description.unwrap_or_default();
        println!("  {}  ★{}\n    {}", hit.full_name, hit.stars, desc);
    }
    Ok(())
}

fn short_commit(commit: &str) -> String {
    commit.chars().take(8).collect()
}

#[cfg(test)]
mod identity_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn cached_repository_entries_add_select_update_and_remove_independently() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let cache = root.join("cache");
        for commit in ["old", "new"] {
            for (subdir, id) in [("a", "alpha"), ("b", "beta")] {
                let package = cache.join(format!("github/owner/repo/{commit}/{subdir}"));
                std::fs::create_dir_all(&package).unwrap();
                std::fs::write(
                    package.join("plugin.toml"),
                    format!("[plugin]\nid = \"{id}\"\nversion = \"1.0.0\"\n"),
                )
                .unwrap();
                std::fs::write(package.join("init.lua"), "return require('rpp').plugin()").unwrap();
            }
        }
        let mut text = "[pack]\nname = \"test\"\n".to_string();
        let mut lock = Lockfile::new();
        for subdir in ["a", "b"] {
            let entry = PluginEntry {
                source: "github:owner/repo.git".into(),
                r#ref: Some(" main ".into()),
                subdir: Some(format!(" ./{subdir}// ")),
                origin: None,
            };
            text = add_plugin(&text, entry.clone()).unwrap();
            assert!(add_plugin(&text, entry).is_err());
            lock.upsert(rpp_fetch::LockedPlugin {
                source: "github:owner/repo".into(),
                ref_: "main".into(),
                requested_ref: Some("main".into()),
                commit: "old".into(),
                subdir: Some(subdir.into()),
            });
        }
        let lock_path = root.join("rpp.lock");
        lock.save(&lock_path).unwrap();
        // Exercise raw modifiers from an existing handwritten configuration as well.
        text = text
            .replace("subdir = \"b\"", "subdir = \" ./b// \"")
            .replace("ref = \"main\"", "ref = \" main \"");
        let config = rpp::config::Config::parse(&text, "rpp.toml").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let resolver = Resolver::new(root)
            .unwrap()
            .with_cache_root(&cache)
            .with_http_config(rpp_fetch::HttpConfig::with_base(format!(
                "http://{}",
                listener.local_addr().unwrap()
            )));
        assert_eq!(
            find_plugin_in(&config.plugins, "beta", &lock, &resolver).unwrap(),
            Some(1)
        );
        assert!(find_plugin_in(&config.plugins, "github:owner/repo", &lock, &resolver).is_err());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut request = [0; 4096];
            let read = stream.read(&mut request).unwrap();
            assert!(String::from_utf8_lossy(&request[..read])
                .starts_with("GET /repos/owner/repo/commits/main "));
            let body = r#"{"sha":"new"}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        assert_eq!(
            update_plugins_with_resolver(&config.plugins, &lock_path, Some("beta"), &resolver)
                .unwrap(),
            (1, true)
        );
        server.join().unwrap();
        let mut lock = Lockfile::load(&lock_path).unwrap();
        assert_eq!(
            lock.get_for("github:owner/repo", Some("main"), Some("a"))
                .unwrap()
                .commit,
            "old"
        );
        assert_eq!(
            lock.get_for("github:owner/repo", Some("main"), Some("b"))
                .unwrap()
                .commit,
            "new"
        );
        let index = find_plugin_in(&config.plugins, "beta", &lock, &resolver)
            .unwrap()
            .unwrap();
        let updated = edit::remove_plugin_at(&text, index).unwrap();
        let remaining = rpp::config::Config::parse(&updated, "rpp.toml").unwrap();
        assert_eq!(remaining.plugins.len(), 1);
        assert_eq!(remaining.plugins[0].subdir.as_deref(), Some("a"));
        lock.remove_for("github:owner/repo", Some("main"), Some("b"))
            .unwrap();
        assert_eq!(lock.plugins().len(), 1);
        assert_eq!(lock.plugins()[0].subdir.as_deref(), Some("a"));
    }
}
