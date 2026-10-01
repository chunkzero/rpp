//! TypeScript editor and type-checker support: the SDK under `.rpp/sdk` and the
//! tsconfig files that map `#rpp` onto it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::project::{resolve_ts_packages, CONFIG_FILE, TS_CONFIG_FILE};

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

const ROOT_TSCONFIG: &str = r#"{
  "extends": "./.rpp/tsconfig.json",
  "include": ["**/*.ts", "**/*.mts", ".rpp/sdk/*.d.ts"],
  "exclude": ["dist", ".rpp/cache"]
}
"#;

const GLOBALS: &str = r#"interface Console {
  log(...data: unknown[]): void;
  info(...data: unknown[]): void;
  warn(...data: unknown[]): void;
  error(...data: unknown[]): void;
  debug(...data: unknown[]): void;
}
declare var console: Console;

declare class TextEncoder {
  readonly encoding: string;
  encode(input?: string): Uint8Array;
  encodeInto(source: string, destination: Uint8Array): { read: number; written: number };
}
declare class TextDecoder {
  constructor(label?: string, options?: { fatal?: boolean; ignoreBOM?: boolean });
  readonly encoding: string;
  decode(input?: ArrayBufferView | ArrayBuffer, options?: { stream?: boolean }): string;
}

declare class URLSearchParams implements Iterable<[string, string]> {
  constructor(init?: string | Record<string, string> | string[][] | URLSearchParams);
  readonly size: number;
  append(name: string, value: string): void;
  delete(name: string, value?: string): void;
  get(name: string): string | null;
  getAll(name: string): string[];
  has(name: string, value?: string): boolean;
  set(name: string, value: string): void;
  sort(): void;
  forEach(callback: (value: string, key: string, parent: URLSearchParams) => void): void;
  entries(): IterableIterator<[string, string]>;
  keys(): IterableIterator<string>;
  values(): IterableIterator<string>;
  [Symbol.iterator](): IterableIterator<[string, string]>;
  toString(): string;
}
declare class URL {
  constructor(url: string, base?: string | URL);
  static canParse(url: string, base?: string | URL): boolean;
  hash: string;
  host: string;
  hostname: string;
  href: string;
  readonly origin: string;
  password: string;
  pathname: string;
  port: string;
  protocol: string;
  search: string;
  readonly searchParams: URLSearchParams;
  username: string;
  toString(): string;
  toJSON(): string;
}

interface Crypto {
  getRandomValues<T extends ArrayBufferView>(array: T): T;
  randomUUID(): string;
}
declare var crypto: Crypto;

declare function atob(data: string): string;
declare function btoa(data: string): string;
"#;

/// The `.rpp/tsconfig.json` contents. `plugin_configs` maps dependency names to their
/// config modules; it is `Some` for `rpp.config.ts` projects, which also map `#rpp/config`.
fn tsconfig(plugin_configs: Option<&BTreeMap<String, PathBuf>>) -> String {
    let mut text = TSCONFIG_HEAD.to_string();
    if let Some(plugin_configs) = plugin_configs {
        text.push_str(",\n      \"#rpp/config\": [\"./sdk/config.ts\"]");
        for (name, path) in plugin_configs {
            let path = path.to_string_lossy().replace('\\', "/");
            text.push_str(&format!(",\n      \"#plugins/{name}\": [{path:?}]"));
        }
    }
    text.push_str(TSCONFIG_TAIL);
    text
}

/// Write the SDK and tsconfig files under `root`, and `tsconfig.json` if absent.
/// Returns whether any file changed.
pub fn write(root: &Path) -> Result<bool> {
    let ts_project = root.join(TS_CONFIG_FILE).is_file() && !root.join(CONFIG_FILE).is_file();
    let plugin_configs = if ts_project {
        let packages = resolve_ts_packages(root)?;
        Some(
            packages
                .into_iter()
                .filter_map(|(name, package)| {
                    let config = package.manifest.config?;
                    Some((name, package.dir.join(config)))
                })
                .collect(),
        )
    } else {
        None
    };
    let rpp_dir = root.join(".rpp");
    let mut changed = false;
    for (path, contents) in rpp::js::SDK_FILES {
        changed |= write_if_changed(&rpp_dir.join("sdk").join(path), contents)?;
    }
    changed |= write_if_changed(&rpp_dir.join("sdk/globals.d.ts"), GLOBALS)?;
    changed |= write_if_changed(
        &rpp_dir.join("tsconfig.json"),
        &tsconfig(plugin_configs.as_ref()),
    )?;

    let root_config = root.join("tsconfig.json");
    if !root_config.exists() {
        std::fs::write(&root_config, ROOT_TSCONFIG)
            .with_context(|| format!("writing {}", root_config.display()))?;
        changed = true;
    }
    Ok(changed)
}

fn write_if_changed(path: &Path, contents: &str) -> Result<bool> {
    if std::fs::read_to_string(path).is_ok_and(|existing| existing == contents) {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(path, contents).with_context(|| format!("writing {}", path.display()))?;
    Ok(true)
}

/// Generate best-effort for build and dev: failures warn instead of aborting.
pub(crate) fn write_best_effort(root: &Path) {
    if let Err(error) = write(root) {
        crate::ui::warn(format!("TypeScript definitions: {error:#}"));
    }
}
