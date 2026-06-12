//! `rpp clean`: remove the build output directory and the `.rpp` cache.

use std::path::Path;

use anyhow::Result;
use rpp::engine::Engine;

use crate::project::Project;
use crate::ui;

/// Run the clean command from the current directory.
pub fn run(dir: &Path) -> Result<()> {
    let project = Project::discover(dir)?;
    let engine = Engine::builder(project.config.clone())
        .project_root(&project.root)
        .build_engine()?;

    ui::phase("Cleaning");
    // The engine removes both the output dir and the entire `.rpp` cache dir.
    engine.clean()?;
    ui::success(format!(
        "Removed {} and {}",
        project.config.build.output.display(),
        ".rpp"
    ));
    Ok(())
}
