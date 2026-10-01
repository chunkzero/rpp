//! Self-contained plugin bundles for publishing.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use oxc::allocator::Allocator;
use oxc::ast::ast::ModuleDeclaration;
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
        let declared = declare(&request.root, &inputs)?;
        if let Some(stub) = config_stub(config, &declared) {
            files.insert("dist/config.d.ts".to_string(), stub);
        }
        declarations = declared
            .into_iter()
            .map(|(path, declaration)| (path, declaration.text))
            .collect();
    }
    Ok(PackOutput {
        files,
        declarations,
    })
}

fn is_typescript(path: &str) -> bool {
    (path.ends_with(".ts") && !path.ends_with(".d.ts"))
        || (path.ends_with(".mts") && !path.ends_with(".d.mts"))
}

/// `types/<path>.d.ts` for a `.ts` file, `.d.mts` for a `.mts` file.
fn declaration_path(relative: &str) -> String {
    match relative.strip_suffix(".mts") {
        Some(stem) => format!("types/{stem}.d.mts"),
        None => format!(
            "types/{}.d.ts",
            relative.strip_suffix(".ts").unwrap_or(relative)
        ),
    }
}

/// `dist/config.d.ts`, re-exporting the config's generated declaration.
fn config_stub(config: &str, declarations: &BTreeMap<String, Declaration>) -> Option<String> {
    let config = config.trim_start_matches("./");
    let declaration = declarations.get(&declaration_path(config))?;
    let target = match config.strip_suffix(".mts") {
        Some(stem) => format!("../types/{stem}.mjs"),
        None => format!(
            "../types/{}.js",
            config.strip_suffix(".ts").unwrap_or(config)
        ),
    };
    let mut stub = format!("export * from \"{target}\";\n");
    if declaration.has_default {
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

/// A generated declaration file.
struct Declaration {
    text: String,
    /// Relative module specifiers the declaration imports or re-exports.
    specifiers: Vec<String>,
    has_default: bool,
}

/// Isolated declarations for every `.ts`/`.mts` file in `inputs` under `root` outside
/// `node_modules`, and for the relative modules those declarations refer to, which
/// bundling never loads when they are only used as types.
fn declare(root: &Path, inputs: &[PathBuf]) -> Result<BTreeMap<String, Declaration>> {
    let root = root.canonicalize()?;
    let mut pending: Vec<PathBuf> = inputs.to_vec();
    let mut seen = BTreeSet::new();
    let mut declarations = BTreeMap::new();
    let mut errors = Vec::new();
    while let Some(path) = pending.pop() {
        let Ok(relative) = path.strip_prefix(&root) else {
            continue;
        };
        let relative_text = relative.to_string_lossy().replace('\\', "/");
        if !is_typescript(&relative_text)
            || relative.components().any(|c| c.as_os_str() == NODE_MODULES)
            || !seen.insert(path.clone())
        {
            continue;
        }
        let source = std::fs::read_to_string(&path)?;
        match declaration(&path, &source) {
            Ok(generated) => {
                let dir = path.parent().unwrap_or(&root);
                pending.extend(
                    generated
                        .specifiers
                        .iter()
                        .filter_map(|specifier| resolve_relative(dir, specifier)),
                );
                declarations.insert(declaration_path(&relative_text), generated);
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
        errors.sort();
        Err(Error::Bundle(format!(
            "cannot generate declarations (isolated declarations require explicit types on exports):\n{}",
            errors.join("\n")
        )))
    }
}

/// The TypeScript file a relative specifier names, as `tsc` resolves it.
fn resolve_relative(dir: &Path, specifier: &str) -> Option<PathBuf> {
    if !specifier.starts_with("./") && !specifier.starts_with("../") {
        return None;
    }
    let base = dir.join(specifier);
    let text = base.to_string_lossy();
    let mut candidates = vec![base.clone()];
    if let Some(stem) = text.strip_suffix(".js") {
        candidates.push(format!("{stem}.ts").into());
    }
    if let Some(stem) = text.strip_suffix(".mjs") {
        candidates.push(format!("{stem}.mts").into());
    }
    for extension in ["ts", "mts"] {
        candidates.push(format!("{text}.{extension}").into());
        candidates.push(base.join(format!("index.{extension}")));
    }
    candidates
        .into_iter()
        .filter(|c| matches!(c.extension().and_then(|e| e.to_str()), Some("ts" | "mts")))
        .find_map(|c| c.canonicalize().ok().filter(|c| c.is_file()))
}

fn declaration(path: &Path, source: &str) -> std::result::Result<Declaration, Vec<String>> {
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(path).unwrap_or_else(|_| SourceType::ts());
    let parsed = Parser::new(&allocator, source, source_type).parse();
    if parsed.diagnostics.has_errors() {
        return Err(parsed.diagnostics.errors().map(describe).collect());
    }
    let generated = IsolatedDeclarations::new(&allocator, IsolatedDeclarationsOptions::default())
        .build(&parsed.program);
    if generated.diagnostics.has_errors() {
        return Err(generated.diagnostics.errors().map(describe).collect());
    }
    let mut specifiers = Vec::new();
    let mut has_default = false;
    for declaration in generated
        .program
        .body
        .iter()
        .filter_map(|statement| statement.as_module_declaration())
    {
        match declaration {
            ModuleDeclaration::ImportDeclaration(d) => specifiers.push(d.source.value.to_string()),
            ModuleDeclaration::ExportAllDeclaration(d) => {
                specifiers.push(d.source.value.to_string());
            }
            ModuleDeclaration::ExportNamedDeclaration(d) => {
                has_default |= d.specifiers.iter().any(|s| s.exported.name() == "default");
            }
            ModuleDeclaration::ExportFromDeclaration(d) => {
                specifiers.push(d.source.value.to_string());
                has_default |= d.specifiers.iter().any(|s| s.exported.name() == "default");
            }
            ModuleDeclaration::ExportDefaultDeclaration(_) => has_default = true,
            _ => {}
        }
    }
    Ok(Declaration {
        text: Codegen::new().build(&generated.program).code,
        specifiers,
        has_default,
    })
}

fn describe(diagnostic: &oxc::diagnostics::OxcDiagnostic) -> String {
    diagnostic.to_string()
}
