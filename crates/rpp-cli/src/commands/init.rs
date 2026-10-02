//! `rpp init`: scaffold a new rpp project (interactive when on a tty).

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::Args;
use rpp_fetch::registry::PACKAGE_MANIFEST;

use crate::project::{legacy_config_error, CONFIG_FILE, LEGACY_CONFIG_FILE};
use crate::ui;

/// Arguments for `rpp init`.
#[derive(Debug, Clone, Default, Args)]
pub struct InitArgs {
    /// Target directory (defaults to the current directory).
    pub dir: Option<PathBuf>,
    /// Pack name (skips the prompt).
    #[arg(long)]
    pub name: Option<String>,
    /// Pack description (skips the prompt).
    #[arg(long)]
    pub description: Option<String>,
    /// Pack format (skips the prompt).
    #[arg(long)]
    pub pack_format: Option<u32>,
    /// Accept defaults without prompting.
    #[arg(short, long)]
    pub yes: bool,
}

/// Default pack_format used when none is supplied (a recent stable value).
const DEFAULT_PACK_FORMAT: u32 = 34;

/// Run the init command.
pub fn run(args: InitArgs) -> Result<()> {
    let target = args.dir.clone().unwrap_or_else(|| PathBuf::from("."));
    std::fs::create_dir_all(&target).with_context(|| format!("creating {}", target.display()))?;

    if target.join(LEGACY_CONFIG_FILE).is_file() {
        return Err(legacy_config_error());
    }
    let config_path = target.join(CONFIG_FILE);
    if config_path.exists() {
        bail!(
            "{} already exists; refusing to overwrite",
            config_path.display()
        );
    }

    ui::intro("Create an rpp project");
    let interactive = !args.yes && ui::is_interactive();

    let default_name = target
        .canonicalize()
        .ok()
        .and_then(|p| p.file_name().map(|s| s.to_string_lossy().to_string()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "my-pack".to_string());

    let name = match args.name {
        Some(n) => n,
        None if interactive => cliclack::input("Pack name")
            .default_input(&default_name)
            .interact()?,
        None => default_name,
    };
    let description = match args.description {
        Some(d) => d,
        None if interactive => cliclack::input("Description")
            .default_input("A Minecraft resource pack")
            .interact()?,
        None => "A Minecraft resource pack".to_string(),
    };
    let pack_format = match args.pack_format {
        Some(f) => f,
        None if interactive => loop {
            let answer: String = cliclack::input("Pack format")
                .default_input(&DEFAULT_PACK_FORMAT.to_string())
                .interact()?;
            match answer.trim().parse::<u32>() {
                Ok(format) if format > 0 => break format,
                _ => ui::warn(format!("`{answer}` is not a valid pack format")),
            }
        },
        None => DEFAULT_PACK_FORMAT,
    };

    scaffold(&target, &name, &description, pack_format)?;

    ui::detail("next: `rpp build`");
    ui::success(format!("Initialized rpp project in {}", target.display()));
    Ok(())
}

/// Write all scaffold files.
fn scaffold(root: &Path, name: &str, description: &str, pack_format: u32) -> Result<()> {
    let destinations = [
        root.join(CONFIG_FILE),
        root.join(PACKAGE_MANIFEST),
        root.join(".gitignore"),
        root.join("src/pack.mcmeta"),
        root.join("plugins/hello").join(PACKAGE_MANIFEST),
        root.join("plugins/hello/src/plugin.ts"),
    ];
    if let Some(path) = destinations.iter().find(|path| path.exists()) {
        bail!("{} already exists; refusing to overwrite", path.display());
    }

    write_file(
        &root.join(CONFIG_FILE),
        &rpp_config(name, description, pack_format),
    )?;
    write_file(&root.join(PACKAGE_MANIFEST), ROOT_MANIFEST)?;
    write_file(&root.join(".gitignore"), GITIGNORE)?;

    let src = root.join("src");
    std::fs::create_dir_all(&src).with_context(|| format!("creating {}", src.display()))?;
    write_file(
        &src.join("pack.mcmeta"),
        &pack_mcmeta(description, pack_format),
    )?;

    // Starter local plugin under plugins/hello/.
    let plugin = root.join("plugins/hello");
    write_file(&plugin.join(PACKAGE_MANIFEST), HELLO_MANIFEST)?;
    write_file(&plugin.join("src/plugin.ts"), HELLO_PLUGIN)?;

    Ok(())
}

fn write_file(path: &Path, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("creating {}", path.display()))?;
    file.write_all(contents.as_bytes())
        .with_context(|| format!("writing {}", path.display()))
}

fn rpp_config(name: &str, description: &str, pack_format: u32) -> String {
    let name = ts_string(name);
    let description = ts_string(description);
    format!(
        r##"import {{ defineConfig, plugin }} from "#rpp/config";

export default defineConfig({{
  pack: {{
    name: {name},
    description: {description},
    packFormat: {pack_format},
  }},
  build: {{ source: "src", output: "dist" }},
  // A starter local plugin. Add more with `rpp add <name>`.
  plugins: [plugin("hello", {{ greeting: "hello" }})],
}});
"##
    )
}

/// A TypeScript string literal for `value`.
fn ts_string(value: &str) -> String {
    serde_json::to_string(value).expect("string serializes")
}

fn pack_mcmeta(description: &str, pack_format: u32) -> String {
    let value = serde_json::json!({
        "pack": {
            "pack_format": pack_format,
            "description": description,
        }
    });
    format!(
        "{}\n",
        serde_json::to_string_pretty(&value).expect("JSON value serializes")
    )
}

const GITIGNORE: &str = "/.rpp/\n/dist/\n";

const ROOT_MANIFEST: &str = r#"{
  "dependencies": {
    "hello": "path:plugins/hello"
  }
}
"#;

const HELLO_MANIFEST: &str = r#"{
  "name": "hello",
  "version": "0.1.0",
  "description": "A starter rpp plugin",
  "entry": "src/plugin.ts"
}
"#;

const HELLO_PLUGIN: &str = r##"import { definePlugin } from "#rpp";

// Processor: minify every JSON / mcmeta file in the pack.
// Generator: emit a tiny build marker listing the pack name and greeting.
const keepNumbers = (_key: string, value: unknown, context?: { source?: string }) =>
  typeof value === "number" && context?.source !== undefined
    ? (JSON as unknown as { rawJSON(text: string): unknown }).rawJSON(context.source)
    : value;

export default definePlugin<{ greeting?: string }>({
  processors: {
    minify: {
      files: ["**/*.json", "**/*.mcmeta"],
      priority: 50,
      run(_ctx, file) {
        try {
          file.text = JSON.stringify(JSON.parse(file.text, keepNumbers));
        } catch {
          // Leave files that are not valid JSON unchanged.
        }
      },
    },
  },
  generate(ctx) {
    ctx.emit("rpp_build.txt", `${ctx.options.greeting ?? "built by rpp"}: ${ctx.pack.name}`);
  },
});
"##;
