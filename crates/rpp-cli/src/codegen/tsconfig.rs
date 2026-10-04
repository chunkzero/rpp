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
    "types": [],
    "paths": {
      "#rpp": ["./sdk/index.ts"]"##;

const TSCONFIG_TAIL: &str = r#"
    }
  }
}
"#;

pub(super) const ROOT_TSCONFIG: &str = r#"{
  "extends": "./.rpp/tsconfig.json",
  "include": ["**/*.ts", "**/*.mts", ".rpp/sdk/*.d.ts", ".rpp/generated/*.d.ts"],
  "exclude": ["dist", ".rpp/cache"]
}
"#;

/// The `.rpp/tsconfig.json` contents. `plugin_configs` maps dependency names to their
/// config modules; it is `Some` for `rpp.config.ts` and plugin projects, which also map
/// `#rpp/config`.
pub(super) fn tsconfig(plugin_configs: Option<&BTreeMap<String, PathBuf>>) -> String {
    let mut text = TSCONFIG_HEAD.to_string();
    if let Some(plugin_configs) = plugin_configs {
        text.push_str(",\n      \"#rpp/config\": [\"./sdk/config.ts\"]");
        for (name, path) in plugin_configs {
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
        }
    }
    text.push_str(TSCONFIG_TAIL);
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_config_maps_to_its_declarations() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config.js");
        std::fs::write(&config, "export default () => {};\n").unwrap();
        let configs = BTreeMap::from([("packed".to_string(), config.clone())]);
        assert!(tsconfig(Some(&configs)).contains("config.js"));
        std::fs::write(
            config.with_extension("d.ts"),
            "export default function(): void;\n",
        )
        .unwrap();
        let generated = tsconfig(Some(&configs));
        assert!(generated.contains("config.d.ts"));
        assert!(!generated.contains("config.js"));
    }

    #[test]
    fn verbatim_windows_paths_are_mapped_as_drive_paths() {
        let configs = BTreeMap::from([(
            "local".to_string(),
            PathBuf::from(r"\\?\C:\plugins\local\config.ts"),
        )]);
        assert!(tsconfig(Some(&configs))
            .contains(r##""#plugins/local": ["C:/plugins/local/config.ts"]"##));
    }
}
