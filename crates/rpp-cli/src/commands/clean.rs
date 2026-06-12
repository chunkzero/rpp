//! `rpp clean`: remove the build output directory and the `.rpp` cache.

use std::path::Path;

use anyhow::Result;

use crate::project::Project;
use crate::ui;

/// Run the clean command from the current directory.
pub fn run(dir: &Path) -> Result<()> {
    let project = Project::discover(dir)?;

    ui::phase("Cleaning");
    project.clean_artifacts()?;
    ui::success(format!(
        "Removed {} and {}",
        project.config.build.output.display(),
        ".rpp"
    ));
    Ok(())
}
