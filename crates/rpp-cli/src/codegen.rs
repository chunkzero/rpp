//! TypeScript editor and type-checker support: the SDK under `.rpp/sdk` and the
//! tsconfig files that map `#rpp` onto it.

use std::path::Path;

use anyhow::{Context, Result};

const TSCONFIG: &str = r##"{
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
    "paths": { "#rpp": ["./sdk/index.ts"] }
  }
}
"##;

const ROOT_TSCONFIG: &str = r#"{
  "extends": "./.rpp/tsconfig.json",
  "include": ["**/*.ts", ".rpp/sdk/*.d.ts"],
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
  encode(input?: string): Uint8Array;
}
declare class TextDecoder {
  constructor(label?: string, options?: { fatal?: boolean; ignoreBOM?: boolean });
  decode(input?: ArrayBufferView | ArrayBuffer): string;
}

declare function queueMicrotask(callback: () => void): void;
declare function structuredClone<T>(value: T): T;
declare function atob(data: string): string;
declare function btoa(data: string): string;
"#;

/// Write the SDK and tsconfig files under `root`, and `tsconfig.json` if absent.
/// Returns whether any file changed.
pub fn write(root: &Path) -> Result<bool> {
    let rpp_dir = root.join(".rpp");
    let mut changed = false;
    for (path, contents) in rpp::js::SDK_FILES {
        changed |= write_if_changed(&rpp_dir.join("sdk").join(path), contents)?;
    }
    changed |= write_if_changed(&rpp_dir.join("sdk/globals.d.ts"), GLOBALS)?;
    changed |= write_if_changed(&rpp_dir.join("tsconfig.json"), TSCONFIG)?;

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
