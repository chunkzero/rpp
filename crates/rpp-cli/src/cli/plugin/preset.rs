use std::{fs::File, io::Write, path::PathBuf};

#[derive(Debug, Clone)]
pub enum PluginPreset {
    Blank,
    LanguageGenerator,
}

impl PluginPreset {
    // I'd implement Display trait here.
    pub fn as_str(&self) -> &'static str {
        match self {
            PluginPreset::Blank => "Blank",
            PluginPreset::LanguageGenerator => "Language Generator",
        }
    }
    // This could implement TryFrom. 
    pub fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(PluginPreset::Blank),
            1 => Some(PluginPreset::LanguageGenerator),
            _ => None,
        }
    }

    pub fn create_preset_files(&self, plugin_dir: &PathBuf) -> anyhow::Result<()> {
        match self {
            PluginPreset::Blank => {
                let mut file = File::create(plugin_dir.join("init.lua"))?;
                file.write_all(include_bytes!("blank_init.lua"))?;
            }
            PluginPreset::LanguageGenerator => {
                let mut file = File::create(plugin_dir.join("init.lua"))?;
                file.write_all(include_bytes!("language_generator_init.lua"))?;
            }
        }

        Ok(())
    }
}
