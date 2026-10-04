//! Generation of `pack.mcmeta` from the `pack` config section.

use serde_json::{json, Map, Value};

use super::{FormatRange, PackConfig};

/// The first resource pack format that reads only `min_format`/`max_format`.
const RANGED_FORMAT: u32 = 65;

impl PackConfig {
    /// The pretty-printed `pack.mcmeta` for this pack.
    pub(crate) fn mcmeta(&self) -> Vec<u8> {
        let mut pack = Map::new();
        pack.insert("description".into(), self.description.clone());
        let FormatRange { min, max } = self.format;
        if min < RANGED_FORMAT {
            pack.insert("pack_format".into(), min.into());
            pack.insert("supported_formats".into(), json!([min, max]));
        }
        insert_range(&mut pack, self.format);

        let mut root = Map::new();
        root.insert("pack".into(), Value::Object(pack));
        if !self.overlays.is_empty() {
            // Pre-65 clients need `formats` on every entry once any overlay targets them.
            let legacy = self
                .overlays
                .iter()
                .any(|overlay| overlay.format.min < RANGED_FORMAT);
            let entries = self
                .overlays
                .iter()
                .map(|overlay| {
                    let mut entry = Map::new();
                    entry.insert("directory".into(), overlay.directory.clone().into());
                    if legacy {
                        let FormatRange { min, max } = overlay.format;
                        entry.insert("formats".into(), json!([min, max]));
                    }
                    insert_range(&mut entry, overlay.format);
                    Value::Object(entry)
                })
                .collect::<Vec<_>>();
            root.insert("overlays".into(), json!({ "entries": entries }));
        }
        if !self.filter.is_empty() {
            let block = self
                .filter
                .iter()
                .map(|pattern| {
                    let mut entry = Map::new();
                    if let Some(namespace) = &pattern.namespace {
                        entry.insert("namespace".into(), namespace.clone().into());
                    }
                    if let Some(path) = &pattern.path {
                        entry.insert("path".into(), path.clone().into());
                    }
                    Value::Object(entry)
                })
                .collect::<Vec<_>>();
            root.insert("filter".into(), json!({ "block": block }));
        }
        if !self.language.is_empty() {
            let languages = self
                .language
                .iter()
                .map(|(code, language)| {
                    let value = json!({
                        "name": language.name,
                        "region": language.region,
                        "bidirectional": language.bidirectional,
                    });
                    (code.clone(), value)
                })
                .collect::<Map<_, _>>();
            root.insert("language".into(), Value::Object(languages));
        }

        let mut bytes =
            serde_json::to_vec_pretty(&Value::Object(root)).expect("JSON value serializes");
        bytes.push(b'\n');
        bytes
    }
}

/// Write `range` as `min_format`/`max_format`, read by format 65 and later. Pre-65 clients
/// ignore these fields.
fn insert_range(object: &mut Map<String, Value>, range: FormatRange) {
    object.insert("min_format".into(), range.min.into());
    object.insert("max_format".into(), range.max.into());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{FilterPattern, LanguageConfig, OverlayConfig};

    fn pack(min: u32, max: u32) -> PackConfig {
        let mut config = crate::config::Config::new("demo", min).pack;
        config.description = "hi".into();
        config.format.max = max;
        config
    }

    fn generated(pack: &PackConfig) -> Value {
        serde_json::from_slice(&pack.mcmeta()).unwrap()
    }

    #[test]
    fn writes_legacy_fields_only_for_pre_65_ranges() {
        let cases = [
            (
                34,
                34,
                json!({
                    "description": "hi", "pack_format": 34, "supported_formats": [34, 34],
                    "min_format": 34, "max_format": 34
                }),
            ),
            (
                46,
                69,
                json!({
                    "description": "hi", "pack_format": 46, "supported_formats": [46, 69],
                    "min_format": 46, "max_format": 69
                }),
            ),
            (
                69,
                75,
                json!({ "description": "hi", "min_format": 69, "max_format": 75 }),
            ),
        ];
        for (min, max, expected) in cases {
            assert_eq!(generated(&pack(min, max)), json!({ "pack": expected }));
        }
    }

    #[test]
    fn writes_overlays_filter_and_languages() {
        let mut config = pack(34, 69);
        config.overlays = vec![
            OverlayConfig {
                directory: "legacy".into(),
                format: FormatRange { min: 34, max: 34 },
            },
            OverlayConfig {
                directory: "modern".into(),
                format: FormatRange { min: 69, max: 75 },
            },
        ];
        config.filter = vec![FilterPattern {
            namespace: Some("minecraft".into()),
            path: None,
        }];
        config.language.insert(
            "qya_aa".into(),
            LanguageConfig {
                name: "Quenya".into(),
                region: "Arda".into(),
                bidirectional: false,
            },
        );
        let value = generated(&config);
        assert_eq!(
            value["overlays"],
            json!({ "entries": [
                { "directory": "legacy", "formats": [34, 34], "min_format": 34, "max_format": 34 },
                { "directory": "modern", "formats": [69, 75], "min_format": 69, "max_format": 75 }
            ] })
        );
        config.overlays.remove(0);
        assert_eq!(
            generated(&config)["overlays"],
            json!({ "entries": [{ "directory": "modern", "min_format": 69, "max_format": 75 }] })
        );
        assert_eq!(
            value["filter"],
            json!({ "block": [{ "namespace": "minecraft" }] })
        );
        assert_eq!(
            value["language"],
            json!({ "qya_aa": { "name": "Quenya", "region": "Arda", "bidirectional": false } })
        );
    }
}
