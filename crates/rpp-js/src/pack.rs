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

use crate::bundle::{build, Built, Chunk, Settings};
use crate::error::{Error, Result};
use crate::BundleRequest;

const NODE_MODULES: &str = "node_modules";

const PLUGIN: &str = "plugin";
const CONFIG: &str = "config";

/// What to pack.
#[derive(Debug, Clone, Default)]
pub struct PackRequest {
    /// The plugin directory.
    pub root: PathBuf,
    /// The plugin entry module, relative to `root`.
    pub plugin: String,
    /// The config entry module, relative to `root`.
    pub config: Option<String>,
    /// Extra entry modules by subpath, relative to `root`.
    pub exports: BTreeMap<String, String>,
    /// The specifier other code uses to import this plugin's config (e.g. `plugin:window`);
    /// it resolves to the `config` entry, and `<self_specifier>/<subpath>` to each export.
    pub self_specifier: Option<String>,
    /// See [`BundleRequest::jsx_import_source`]; the import source stays an import.
    pub jsx_import_source: Option<String>,
}

/// The files of a packed plugin, as text keyed by archive path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackOutput {
    /// The archive path of the bundled plugin entry, `dist/plugin.js`.
    pub plugin: String,
    /// The archive path of the bundled config entry, `dist/config.js`.
    pub config: Option<String>,
    /// The archive path of each bundled export by subpath, `dist/exports/<subpath>.js`.
    pub exports: BTreeMap<String, String>,
    /// `dist/**.js` chunks, their `.js.map` files, and a `.d.ts` stub beside each TypeScript
    /// config or export entry.
    pub files: BTreeMap<String, String>,
    /// `types/**.d.ts`, one per TypeScript file of the config's and exports' import graphs
    /// outside `node_modules`.
    pub declarations: BTreeMap<String, String>,
}

/// Bundle a plugin's entries into self-contained ES modules with npm dependencies
/// inlined from `node_modules` (including hoisted ones outside `root`).
///
/// `rpp` and `rpp:*` stay as imports. Source map sources are relative to the map
/// file (`../src/plugin.ts`), with dependency files under `node_modules/`.
///
/// # Errors
///
/// [`Error::Invalid`] for an empty plugin entry; [`Error::Bundle`] for bundling
/// failures or a config module that does not support isolated declarations.
pub fn pack(request: &PackRequest) -> Result<PackOutput> {
    if request.plugin.is_empty() {
        return Err(Error::Invalid("pack requires a plugin entry".to_string()));
    }
    let bundle_request = BundleRequest {
        root: request.root.clone(),
        jsx_import_source: request.jsx_import_source.clone(),
        ..Default::default()
    };
    let mut settings = settings(request);
    let modules: Vec<(String, &str)> = request
        .config
        .iter()
        .map(|config| (CONFIG.to_string(), config.as_str()))
        .chain(
            request
                .exports
                .iter()
                .map(|(subpath, module)| (format!("exports/{subpath}"), module.as_str())),
        )
        .collect();
    let inputs = modules
        .iter()
        .map(|(name, module)| input(name, module))
        .chain([input(PLUGIN, &request.plugin)])
        .collect();
    let built = build(&bundle_request, &settings, inputs)?;
    let mut files = chunk_files(built.chunks)?;

    settings.split = false;
    let mut declared_inputs = BTreeSet::new();
    let typed: Vec<_> = modules.iter().filter(|(_, m)| is_typescript(m)).collect();
    for (name, module) in &typed {
        let Built { inputs, .. } = build(&bundle_request, &settings, vec![input(name, module)])?;
        declared_inputs.extend(inputs);
    }
    let declared_inputs: Vec<_> = declared_inputs.into_iter().collect();
    let declared = declare(&request.root, &declared_inputs)?;
    for (name, module) in typed {
        if let Some(stub) = entry_stub(name, module, &declared) {
            files.insert(format!("dist/{name}.d.ts"), stub);
        }
    }
    let declarations = declared
        .into_iter()
        .map(|(path, declaration)| (path, declaration.text))
        .collect();
    Ok(PackOutput {
        plugin: entry_path(PLUGIN),
        config: request.config.as_ref().map(|_| entry_path(CONFIG)),
        exports: request
            .exports
            .keys()
            .map(|subpath| (subpath.clone(), entry_path(&format!("exports/{subpath}"))))
            .collect(),
        files,
        declarations,
    })
}

fn settings(request: &PackRequest) -> Settings {
    let mut settings = Settings {
        externals: vec!["rpp".to_string(), "rpp:".to_string()],
        allow_node_modules: true,
        main_fields: Some(vec!["module".to_string(), "main".to_string()]),
        split: true,
        ..Default::default()
    };
    if let Some(specifier) = &request.self_specifier {
        if let Some(config) = &request.config {
            settings.aliases.insert(specifier.clone(), config.clone());
        }
        for (subpath, module) in &request.exports {
            settings
                .aliases
                .insert(format!("{specifier}/{subpath}"), module.clone());
        }
    }
    settings
}

fn input(name: &str, entry: &str) -> InputItem {
    InputItem {
        name: Some(name.to_string()),
        import: entry.to_string(),
    }
}

/// Where the split build emits the entry chunk `name`.
fn entry_path(name: &str) -> String {
    format!("dist/{name}.js")
}

/// `dist/` files for each chunk: its code, linked to a `.js.map` beside it when it has one.
fn chunk_files(chunks: Vec<Chunk>) -> Result<BTreeMap<String, String>> {
    let mut files = BTreeMap::new();
    for chunk in chunks {
        let mut code = chunk.code;
        if !code.ends_with('\n') {
            code.push('\n');
        }
        if let Some(source_map) = &chunk.source_map {
            let map_path = format!("{}.map", chunk.file_name);
            let map_name = map_path.rsplit('/').next().unwrap_or(&map_path);
            code.push_str(&format!("//# sourceMappingURL={map_name}\n"));
            files.insert(
                format!("dist/{map_path}"),
                relative_to_dist(source_map, &chunk.file_name)?,
            );
        }
        files.insert(format!("dist/{}", chunk.file_name), code);
    }
    Ok(files)
}

fn is_typescript(path: &str) -> bool {
    (path.ends_with(".ts") && !path.ends_with(".d.ts"))
        || (path.ends_with(".mts") && !path.ends_with(".d.mts"))
        || path.ends_with(".tsx")
}

/// The path without its `.ts` or `.tsx` extension.
fn ts_stem(path: &str) -> &str {
    path.strip_suffix(".tsx")
        .or_else(|| path.strip_suffix(".ts"))
        .unwrap_or(path)
}

/// `types/<path>.d.ts` for a `.ts` or `.tsx` file, `.d.mts` for a `.mts` file.
fn declaration_path(relative: &str) -> String {
    match relative.strip_suffix(".mts") {
        Some(stem) => format!("types/{stem}.d.mts"),
        None => format!("types/{}.d.ts", ts_stem(relative)),
    }
}

/// `dist/<name>.d.ts`, re-exporting the generated declaration of the entry `module`.
fn entry_stub(
    name: &str,
    module: &str,
    declarations: &BTreeMap<String, Declaration>,
) -> Option<String> {
    let module = module.trim_start_matches("./");
    let declaration = declarations.get(&declaration_path(module))?;
    let up = "../".repeat(name.matches('/').count() + 1);
    let target = match module.strip_suffix(".mts") {
        Some(stem) => format!("{up}types/{stem}.mjs"),
        None => format!("{up}types/{}.js", ts_stem(module)),
    };
    let mut stub = format!("export * from \"{target}\";\n");
    if declaration.has_default {
        stub.push_str(&format!("export {{ default }} from \"{target}\";\n"));
    }
    Some(stub)
}

/// Rewrites the map's sources (relative to the plugin directory) to be relative to
/// `dist/<file_name>`.
fn relative_to_dist(source_map: &str, file_name: &str) -> Result<String> {
    let up = "../".repeat(file_name.matches('/').count() + 1);
    let mut map: serde_json::Value = serde_json::from_str(source_map)
        .map_err(|e| Error::Bundle(format!("invalid source map: {e}")))?;
    if let Some(sources) = map["sources"].as_array_mut() {
        for source in sources {
            if let Some(text) = source.as_str() {
                *source = format!("{up}{text}").into();
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

/// Isolated declarations for every `.ts`/`.tsx`/`.mts` file in `inputs` under `root` outside
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
        candidates.push(format!("{stem}.tsx").into());
    }
    if let Some(stem) = text.strip_suffix(".jsx") {
        candidates.push(format!("{stem}.tsx").into());
        candidates.push(format!("{stem}.ts").into());
    }
    if let Some(stem) = text.strip_suffix(".mjs") {
        candidates.push(format!("{stem}.mts").into());
    }
    for extension in ["ts", "tsx", "mts"] {
        candidates.push(format!("{text}.{extension}").into());
    }
    for extension in ["ts", "tsx", "mts"] {
        candidates.push(base.join(format!("index.{extension}")));
    }
    candidates
        .into_iter()
        .filter(|c| {
            matches!(
                c.extension().and_then(|e| e.to_str()),
                Some("ts" | "tsx" | "mts")
            )
        })
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
