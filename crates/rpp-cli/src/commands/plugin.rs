//! `rpp plugin ...`: manage `[[plugin]]` entries in `rpp.toml` (via `toml_edit`,
//! preserving formatting and comments) and the `rpp.lock` pins (via `rpp-fetch`).

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use rpp_fetch::{search, LockedPlugin, Lockfile, PluginSource, Resolver};

use crate::project::{validate_plugin_dir, Project};
use crate::ui;

mod edit;

pub use edit::{add_plugin, remove_plugin, PluginEntry};

/// The plugin subcommand to run.
#[derive(Debug, Clone)]
pub enum PluginCommand {
    /// Add a plugin.
    Add {
        /// Source string (`path:...` or `github:owner/repo`).
        source: String,
        /// Optional git ref for GitHub sources.
        r#ref: Option<String>,
        /// Optional subdir within a GitHub repo.
        subdir: Option<String>,
    },
    /// Remove a plugin by id (or by source string).
    Remove {
        /// The plugin id or source string to remove.
        id: String,
    },
    /// List configured plugins.
    List,
    /// Re-resolve plugins (ignoring existing pins), updating the lockfile.
    Update {
        /// An optional single plugin (id or source) to update.
        id: Option<String>,
    },
    /// Search GitHub for `rpp-plugin`-topic repositories.
    Search {
        /// The search query.
        query: String,
    },
}

/// Run a plugin subcommand from the current directory.
pub fn run(dir: &Path, command: PluginCommand) -> Result<()> {
    match command {
        PluginCommand::Add {
            source,
            r#ref,
            subdir,
        } => add(dir, &source, r#ref.as_deref(), subdir.as_deref()),
        PluginCommand::Remove { id } => remove(dir, &id),
        PluginCommand::List => list(dir),
        PluginCommand::Update { id } => update(dir, id.as_deref()),
        PluginCommand::Search { query } => run_search(&query),
    }
}

fn add(dir: &Path, source: &str, ref_: Option<&str>, subdir: Option<&str>) -> Result<()> {
    let project = Project::discover(dir)?;

    // Parse + resolve immediately so we can validate and surface id/version.
    let parsed = PluginSource::parse(source, ref_, subdir)
        .with_context(|| format!("invalid plugin source `{source}`"))?;
    let canonical = parsed.canonical();

    let resolver = Resolver::new(&project.root).context("initializing resolver")?;
    let lock_path = project.lock_path();
    let mut lock = Lockfile::load(&lock_path)?;

    let resolved = resolver
        .resolve(&parsed, lock.get_for(&canonical, ref_, subdir))
        .with_context(|| format!("resolving plugin `{source}`"))?;

    let (id, version) = validate_plugin_dir(&resolved.root)?;

    // Edit rpp.toml, preserving formatting.
    let config_path = project.config_path();
    let text = std::fs::read_to_string(&config_path)?;
    let updated = add_plugin(
        &text,
        PluginEntry {
            source: source.to_string(),
            r#ref: ref_.map(str::to_string),
            subdir: subdir.map(str::to_string),
        },
    )?;
    std::fs::write(&config_path, &updated)?;

    // Record any pin.
    if let Some(pin) = &resolved.pinned {
        lock.upsert(LockedPlugin {
            source: canonical,
            ref_: pin.ref_.clone(),
            commit: pin.commit.clone(),
            subdir: subdir.map(str::to_string),
        });
        lock.save(&lock_path)?;
    }

    ui::success(format!("Added plugin `{id}` v{version}"));
    Ok(())
}

fn remove(dir: &Path, id_or_source: &str) -> Result<()> {
    let project = Project::discover(dir)?;
    let config_path = project.config_path();
    let text = std::fs::read_to_string(&config_path)?;
    let lock_path = project.lock_path();
    let mut lock = Lockfile::load(&lock_path)?;
    let resolver = Resolver::new(&project.root).ok();

    let resolved_source = project.config.plugins.iter().find_map(|plugin| {
        if plugin.source == id_or_source {
            return Some(plugin.source.clone());
        }
        let parsed = PluginSource::parse(
            &plugin.source,
            plugin.r#ref.as_deref(),
            plugin.subdir.as_deref(),
        )
        .ok()?;
        let locked = lock
            .get_for(
                &parsed.canonical(),
                plugin.r#ref.as_deref(),
                plugin.subdir.as_deref(),
            )
            .cloned();
        if matches!(&parsed, PluginSource::GitHub { .. }) && locked.is_none() {
            return None;
        }
        let resolved = resolver.as_ref()?.resolve(&parsed, locked.as_ref()).ok()?;
        let (id, _) = validate_plugin_dir(&resolved.root).ok()?;
        (id == id_or_source).then(|| plugin.source.clone())
    });
    let target = resolved_source.as_deref().unwrap_or(id_or_source);

    let (updated, removed_source) = remove_plugin(&text, target, &project.root)?;
    let removed_source =
        removed_source.ok_or_else(|| anyhow!("no plugin matching `{id_or_source}` found"))?;
    std::fs::write(&config_path, &updated)?;

    // Drop the lockfile pin if present.
    if let Ok(parsed) = PluginSource::parse(&removed_source, None, None) {
        if lock.remove(&parsed.canonical()).is_some() {
            lock.save(&lock_path)?;
        }
    }

    ui::success(format!("Removed plugin `{id_or_source}`"));
    Ok(())
}

fn list(dir: &Path) -> Result<()> {
    let project = Project::discover(dir)?;
    let lock = Lockfile::load(&project.lock_path())?;

    if project.config.plugins.is_empty() {
        ui::detail("no plugins configured");
        return Ok(());
    }

    let resolver = Resolver::new(&project.root).ok();
    for plugin in &project.config.plugins {
        let parsed = PluginSource::parse(
            &plugin.source,
            plugin.r#ref.as_deref(),
            plugin.subdir.as_deref(),
        );
        let pin = parsed
            .as_ref()
            .ok()
            .and_then(|p| {
                lock.get_for(
                    &p.canonical(),
                    plugin.r#ref.as_deref(),
                    plugin.subdir.as_deref(),
                )
            })
            .map(|l| format!("{}@{}", l.ref_, short_commit(&l.commit)));

        // Best-effort id/version by resolving from cache/path.
        let version = parsed
            .as_ref()
            .ok()
            .zip(resolver.as_ref())
            .and_then(|(p, r)| {
                let locked = lock
                    .get_for(
                        &p.canonical(),
                        plugin.r#ref.as_deref(),
                        plugin.subdir.as_deref(),
                    )
                    .cloned();
                if matches!(p, PluginSource::GitHub { .. }) && locked.is_none() {
                    return None;
                }
                r.resolve(p, locked.as_ref()).ok()
            })
            .and_then(|resolved| validate_plugin_dir(&resolved.root).ok());

        let id_ver = match version {
            Some((id, ver)) => format!("{id} v{ver}"),
            None => "(unresolved)".to_string(),
        };
        match pin {
            Some(pin) => println!("  {id_ver}  [{}]  pin={pin}", plugin.source),
            None => println!("  {id_ver}  [{}]", plugin.source),
        }
    }
    Ok(())
}

fn update(dir: &Path, id_or_source: Option<&str>) -> Result<()> {
    let project = Project::discover(dir)?;
    let resolver = Resolver::new(&project.root).context("initializing resolver")?;
    let lock_path = project.lock_path();
    let mut lock = Lockfile::load(&lock_path)?;
    let mut changed = 0usize;
    let mut matched = false;

    for plugin in &project.config.plugins {
        let parsed = match PluginSource::parse(
            &plugin.source,
            plugin.r#ref.as_deref(),
            plugin.subdir.as_deref(),
        ) {
            Ok(p) => p,
            Err(_) => continue,
        };

        // When a specific target is named, skip non-matching entries.
        if let Some(target) = id_or_source {
            if plugin.source != target && parsed.canonical() != target {
                // Could still match by id; check by resolving (cheap if cached).
                let matches_id = lock
                    .get_for(
                        &parsed.canonical(),
                        plugin.r#ref.as_deref(),
                        plugin.subdir.as_deref(),
                    )
                    .and_then(|l| resolver.resolve(&parsed, Some(l)).ok())
                    .and_then(|r| validate_plugin_dir(&r.root).ok())
                    .map(|(id, _)| id == target)
                    .unwrap_or(false);
                if !matches_id {
                    continue;
                }
            }
        }
        matched = true;

        // Path sources are never locked; nothing to update.
        if matches!(parsed, PluginSource::Path { .. }) {
            continue;
        }

        // Re-resolve ignoring the existing pin (force network).
        let resolved = resolver
            .resolve(&parsed, None)
            .with_context(|| format!("updating plugin `{}`", plugin.source))?;
        if let Some(pin) = &resolved.pinned {
            let prev = lock.upsert(LockedPlugin {
                source: parsed.canonical(),
                ref_: pin.ref_.clone(),
                commit: pin.commit.clone(),
                subdir: plugin.subdir.clone(),
            });
            if prev.map(|p| p.commit) != Some(pin.commit.clone()) {
                changed += 1;
                ui::detail(format!(
                    "{} -> {}",
                    plugin.source,
                    short_commit(&pin.commit)
                ));
            }
        }
    }

    if changed > 0 {
        lock.save(&lock_path)?;
    }
    if id_or_source.is_some() && !matched {
        return Err(anyhow!(
            "no plugin matching `{}` found",
            id_or_source.unwrap_or_default()
        ));
    }
    ui::success(format!(
        "{changed} plugin{} updated",
        if changed == 1 { "" } else { "s" }
    ));
    Ok(())
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
