//! The `.rpp/tsconfig.json` and root `tsconfig.json` contents.

use std::collections::BTreeMap;
use std::path::PathBuf;

const TSCONFIG_HEAD: &str = r##"{
  "compilerOptions": {
    "target": "ES2023",
    "lib": ["ES2023"],
    "module": "ESNext",
    "moduleResolution": "Bundler",
    "strict": true,
    "noEmit": true,
    "allowImportingTsExtensions": true,
    "verbatimModuleSyntax": true,
    "isolatedModules": true,
    "skipLibCheck": true,
    "types": [],"##;

const PATHS_HEAD: &str = r##"
    "paths": {
      "#rpp": ["./sdk/index.ts"]"##;

const TSCONFIG_TAIL: &str = r#"
    }
  }
}
"#;

pub(super) const ROOT_TSCONFIG: &str = r#"{
  "extends": "./.rpp/tsconfig.json",
  "include": ["**/*.ts", "**/*.mts", "**/*.tsx", ".rpp/sdk/*.d.ts", ".rpp/generated/*.d.ts"],
  "exclude": ["dist", ".rpp/cache"]
}
"#;

/// A dependency's config module, as `#plugins/<name>` resolves it.
pub(super) struct PluginConfig {
    pub(super) path: PathBuf,
    /// Whether the module is also the plugin's JSX runtime, `#plugins/<name>/jsx-runtime`.
    pub(super) jsx: bool,
}

/// The `.rpp/tsconfig.json` contents. `plugin_configs` maps dependency names to their
/// config modules; it is `Some` for `rpp.config.ts` and plugin projects, which also map
/// `#rpp/config`. When exactly one dependency provides a JSX runtime, `.tsx` files use it.
pub(super) fn tsconfig(plugin_configs: Option<&BTreeMap<String, PluginConfig>>) -> String {
    let mut text = TSCONFIG_HEAD.to_string();
    let mut jsx = plugin_configs
        .into_iter()
        .flatten()
        .filter(|(_, config)| config.jsx);
    if let Some((name, _)) = jsx.next() {
        text.push_str("\n    \"jsx\": \"react-jsx\",");
        if jsx.next().is_none() {
            text.push_str(&format!("\n    \"jsxImportSource\": \"#plugins/{name}\","));
        }
    }
    text.push_str(PATHS_HEAD);
    if let Some(plugin_configs) = plugin_configs {
        text.push_str(",\n      \"#rpp/config\": [\"./sdk/config.ts\"]");
        for (name, PluginConfig { path, jsx }) in plugin_configs {
            let declaration = match path.extension().and_then(|extension| extension.to_str()) {
                Some("js") => path.with_extension("d.ts"),
                Some("mjs") => path.with_extension("d.mts"),
                Some("cjs") => path.with_extension("d.cts"),
                _ => path.clone(),
            };
            let path = if declaration.is_file() {
                &declaration
            } else {
                path
            };
            let path = path.to_string_lossy();
            // tsc cannot resolve Windows verbatim paths, which canonicalization produces.
            let path = path
                .strip_prefix(r"\\?\")
                .unwrap_or(&path)
                .replace('\\', "/");
            text.push_str(&format!(",\n      \"#plugins/{name}\": [{path:?}]"));
            if *jsx {
                text.push_str(&format!(
                    ",\n      \"#plugins/{name}/jsx-runtime\": [{path:?}]"
                ));
            }
        }
    }
    text.push_str(TSCONFIG_TAIL);
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin(path: PathBuf, jsx: bool) -> PluginConfig {
        PluginConfig { path, jsx }
    }

    #[test]
    fn a_single_jsx_plugin_is_the_jsx_import_source() {
        let mut configs = BTreeMap::from([
            (
                "ui".to_string(),
                plugin(PathBuf::from("/ui/config.ts"), true),
            ),
            (
                "other".to_string(),
                plugin(PathBuf::from("/other/config.ts"), false),
            ),
        ]);
        let generated = tsconfig(Some(&configs));
        serde_json::from_str::<serde_json::Value>(&generated).unwrap();
        assert!(generated.contains(r##""jsxImportSource": "#plugins/ui""##));
        assert!(generated.contains(r##""#plugins/ui/jsx-runtime": ["/ui/config.ts"]"##));
        assert!(!generated.contains("#plugins/other/jsx-runtime"));

        configs.insert(
            "more".to_string(),
            plugin(PathBuf::from("/more/config.ts"), true),
        );
        let generated: serde_json::Value = serde_json::from_str(&tsconfig(Some(&configs))).unwrap();
        assert_eq!(generated["compilerOptions"]["jsx"], "react-jsx");
        assert!(generated["compilerOptions"]
            .get("jsxImportSource")
            .is_none());
        for configs in [None, Some(&BTreeMap::new())] {
            let generated: serde_json::Value = serde_json::from_str(&tsconfig(configs)).unwrap();
            assert!(generated["compilerOptions"].get("jsx").is_none());
        }
    }

    #[test]
    fn packed_config_maps_to_its_declarations() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config.js");
        std::fs::write(&config, "export default () => {};\n").unwrap();
        let configs = BTreeMap::from([("packed".to_string(), plugin(config.clone(), true))]);
        assert!(tsconfig(Some(&configs)).contains("config.js"));
        std::fs::write(
            config.with_extension("d.ts"),
            "export default function(): void;\n",
        )
        .unwrap();
        let generated = tsconfig(Some(&configs));
        assert!(generated.contains("config.d.ts"));
        assert!(!generated.contains("config.js"));
        let generated: serde_json::Value = serde_json::from_str(&generated).unwrap();
        assert_eq!(
            generated["compilerOptions"]["paths"]["#plugins/packed/jsx-runtime"],
            generated["compilerOptions"]["paths"]["#plugins/packed"]
        );
    }

    #[test]
    fn verbatim_windows_paths_are_mapped_as_drive_paths() {
        let configs = BTreeMap::from([(
            "local".to_string(),
            plugin(PathBuf::from(r"\\?\C:\plugins\local\config.ts"), false),
        )]);
        assert!(tsconfig(Some(&configs))
            .contains(r##""#plugins/local": ["C:/plugins/local/config.ts"]"##));
    }
}
