//! Self-contained plugin bundles for publishing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use oxc::allocator::Allocator;
use oxc::codegen::Codegen;
use oxc::isolated_declarations::{IsolatedDeclarations, IsolatedDeclarationsOptions};
use oxc::parser::Parser;
use oxc::span::SourceType;
use rolldown::InputItem;

use crate::bundle::{build, Built, Settings};
use crate::error::{Error, Result};
use crate::BundleRequest;

const NODE_MODULES: &str = "node_modules";

/// What to pack.
#[derive(Debug, Clone, Default)]
pub struct PackRequest {
    /// The plugin directory.
    pub root: PathBuf,
    /// Entry modules relative to `root`, keyed `plugin` (required) or `config`.
    pub entries: BTreeMap<String, String>,
    /// The specifier other plugins use to import this plugin's config (e.g.
    /// `#plugins/window`); it resolves to the `config` entry.
    pub self_specifier: Option<String>,
}

/// The files of a packed plugin, as text keyed by archive path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackOutput {
    /// `dist/*.js` chunks, their `.js.map` files and `dist/config.d.ts`.
    pub files: BTreeMap<String, String>,
    /// `types/**.d.ts`, one per TypeScript file of the config's import graph outside
    /// `node_modules`.
    pub declarations: BTreeMap<String, String>,
}

/// Bundle a plugin's entries into self-contained ES modules with npm dependencies
/// inlined from `node_modules` (including hoisted ones outside `root`).
///
/// `#rpp` and `#rpp/*` stay as imports. Source map sources are relative to the map
/// file (`../src/plugin.ts`), with dependency files under `node_modules/`.
///
/// # Errors
///
/// [`Error::Bundle`] for bundling failures, an unknown or missing entry, or a config
/// module that does not support isolated declarations.
pub fn pack(request: &PackRequest) -> Result<PackOutput> {
    if !request.entries.contains_key("plugin") {
        return Err(Error::Bundle("pack requires a `plugin` entry".to_string()));
    }
    if let Some(name) = request
        .entries
        .keys()
        .find(|k| !matches!(k.as_str(), "plugin" | "config"))
    {
        return Err(Error::Bundle(format!("unknown pack entry `{name}`")));
    }
    let bundle_request = BundleRequest {
        root: request.root.clone(),
        ..Default::default()
    };
    let config = request.entries.get("config");
    let mut settings = Settings {
        externals: vec!["#rpp".to_string(), "#rpp/".to_string()],
        allow_node_modules: true,
        main_fields: Some(vec!["module".to_string(), "main".to_string()]),
        split: true,
        ..Default::default()
    };
    if let (Some(specifier), Some(config)) = (&request.self_specifier, config) {
        settings.aliases.insert(specifier.clone(), config.clone());
    }
    let inputs = request
        .entries
        .iter()
        .map(|(name, entry)| InputItem {
            name: Some(name.clone()),
            import: entry.clone(),
        })
        .collect();
    let built = build(&bundle_request, &settings, inputs)?;

    let mut files = BTreeMap::new();
    for chunk in built.chunks {
        let map_name = format!("{}.map", chunk.file_name);
        let mut code = chunk.code;
        if !code.ends_with('\n') {
            code.push('\n');
        }
        code.push_str(&format!("//# sourceMappingURL={map_name}\n"));
        files.insert(format!("dist/{}", chunk.file_name), code);
        files.insert(
            format!("dist/{map_name}"),
            relative_to_dist(&chunk.source_map)?,
        );
    }

    let mut declarations = BTreeMap::new();
    if let Some(config) = config.filter(|c| is_typescript(c)) {
        settings.split = false;
        let inputs = vec![InputItem {
            name: Some("config".to_string()),
            import: config.clone(),
        }];
        let Built { inputs, .. } = build(&bundle_request, &settings, inputs)?;
        declarations = declare(&request.root, &inputs)?;
        let stub = config_stub(config, &declarations);
        if let Some(stub) = stub {
            files.insert("dist/config.d.ts".to_string(), stub);
        }
    }
    Ok(PackOutput {
        files,
        declarations,
    })
}

fn is_typescript(path: &str) -> bool {
    path.ends_with(".ts") && !path.ends_with(".d.ts")
}

fn declaration_path(relative: &str) -> String {
    format!("types/{}.d.ts", relative.trim_end_matches(".ts"))
}

/// `dist/config.d.ts`, re-exporting the config's generated declaration.
fn config_stub(config: &str, declarations: &BTreeMap<String, String>) -> Option<String> {
    let declaration = declarations.get(&declaration_path(config.trim_start_matches("./")))?;
    let target = format!(
        "../types/{}.js",
        config.trim_start_matches("./").trim_end_matches(".ts")
    );
    let mut stub = format!("export * from \"{target}\";\n");
    if declaration.contains("export default") {
        stub.push_str(&format!("export {{ default }} from \"{target}\";\n"));
    }
    Some(stub)
}

/// Rewrites the map's sources (relative to the plugin directory) to be relative to
/// `dist/`.
fn relative_to_dist(source_map: &str) -> Result<String> {
    let mut map: serde_json::Value = serde_json::from_str(source_map)
        .map_err(|e| Error::Bundle(format!("invalid source map: {e}")))?;
    if let Some(sources) = map["sources"].as_array_mut() {
        for source in sources {
            if let Some(text) = source.as_str() {
                *source = format!("../{text}").into();
            }
        }
    }
    Ok(map.to_string())
}

/// Isolated declarations for every `.ts` file in `inputs` under `root`, outside
/// `node_modules`.
fn declare(root: &Path, inputs: &[PathBuf]) -> Result<BTreeMap<String, String>> {
    let root = root.canonicalize()?;
    let mut declarations = BTreeMap::new();
    let mut errors = Vec::new();
    for path in inputs {
        let Ok(relative) = path.strip_prefix(&root) else {
            continue;
        };
        let relative_text = relative.to_string_lossy().replace('\\', "/");
        if !is_typescript(&relative_text)
            || relative.components().any(|c| c.as_os_str() == NODE_MODULES)
        {
            continue;
        }
        let source = std::fs::read_to_string(path)?;
        match declaration(&source) {
            Ok(text) => {
                declarations.insert(declaration_path(&relative_text), text);
            }
            Err(messages) => {
                errors.extend(
                    messages
                        .into_iter()
                        .map(|m| format!("{relative_text}: {m}")),
                );
            }
        }
    }
    if errors.is_empty() {
        Ok(declarations)
    } else {
        Err(Error::Bundle(format!(
            "cannot generate declarations (isolated declarations require explicit types on exports):\n{}",
            errors.join("\n")
        )))
    }
}

fn declaration(source: &str) -> std::result::Result<String, Vec<String>> {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, SourceType::ts()).parse();
    if parsed.diagnostics.has_errors() {
        return Err(parsed.diagnostics.errors().map(describe).collect());
    }
    let generated = IsolatedDeclarations::new(&allocator, IsolatedDeclarationsOptions::default())
        .build(&parsed.program);
    if generated.diagnostics.has_errors() {
        return Err(generated.diagnostics.errors().map(describe).collect());
    }
    Ok(Codegen::new().build(&generated.program).code)
}

fn describe(diagnostic: &oxc::diagnostics::OxcDiagnostic) -> String {
    diagnostic.to_string()
}
