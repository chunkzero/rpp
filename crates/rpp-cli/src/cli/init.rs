use std::path::PathBuf;
use crate::DEFAULT_CONFIG_PATH;

pub(super) fn init() -> anyhow::Result<()> {
    let config_path = PathBuf::from(DEFAULT_CONFIG_PATH);
    
    if config_path.exists() {
        tracing::warn!("An existing configuration file already exists. Exiting");
        return Ok(())
    }
    
    Ok(())
}