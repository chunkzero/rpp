// Project configuration types, imported as `#rpp/config` from `rpp.config.ts` and
// from plugins' config modules.

export interface PackConfig {
  /** Pack name; used for the zip filename. */
  name: string;
  description?: string;
  /** Validated against `pack.mcmeta` when present. */
  packFormat?: number;
}

export interface LimitsConfig {
  /** Heap limit per plugin runtime, in megabytes. */
  memoryLimitMb?: number;
  /** Wall-clock limit per plugin call, in seconds. */
  executionDeadlineSeconds?: number;
}

export interface SquashConfig {
  enabled?: boolean;
  engine?: "builtin" | "packsquash";
  json?: boolean;
  png?: boolean | "off" | "fast" | "max";
  zip?: boolean;
  /** Globs removed from the squashed pack. */
  strip?: string[];
  packsquashBinary?: string;
  packsquashOptions?: string;
}

export interface BuildConfig {
  /** Pack source directory. Defaults to `src`. */
  source?: string;
  /** Output directory. Defaults to `dist`. */
  output?: string;
  /** Worker threads; 0 uses available parallelism. */
  workers?: number;
  /** Limits for TypeScript plugins. */
  limits?: LimitsConfig;
  /** Limits for WASM components. */
  wasm?: LimitsConfig;
  squash?: SquashConfig;
}

export interface DevConfig {
  host?: string;
  port?: number;
  open?: boolean;
}

export interface Permissions {
  /** Programs `process.run` may start. */
  process?: string[];
  /** Environment variables visible to processes and components. */
  environment?: string[];
  /** Project-relative directories readable by components. */
  read?: string[];
  /** Project-relative directories writable by components. */
  write?: string[];
  network?: boolean;
  /** Real time instead of a fixed clock. */
  clocks?: boolean;
  /** Real randomness instead of a seeded generator. */
  random?: boolean;
  stdio?: boolean;
}

/** Host access granted to one plugin. Any permission requires `security: "trusted"`. */
export interface Access {
  security?: "sandboxed" | "trusted";
  permissions?: Permissions;
  /** Named project-relative directories the plugin may write outside the pack. */
  outputs?: Record<string, string>;
}

/** One configured plugin: a dependency name from `rpp.json` plus its options. */
export interface PluginEntry extends Access {
  plugin: string;
  options?: unknown;
}

export interface Config {
  pack: PackConfig;
  build?: BuildConfig;
  dev?: DevConfig;
  /** Plugins in order; earlier plugins win processor priority ties. */
  plugins?: PluginEntry[];
}

export function defineConfig(config: Config): Config {
  return config;
}

/** Configure a plugin that ships no config module. Options are untyped. */
export function plugin(name: string, options?: unknown, access?: Access): PluginEntry {
  return { ...access, plugin: name, options };
}

export interface PluginConfigOptions<Options> {
  /** Fill in defaults or reshape options before they reach the plugin. */
  normalize?(options: Options): unknown;
  /** Throw, or return messages, when options are invalid. */
  validate?(options: Options): string[] | void;
}

/**
 * Declare a plugin's config factory. A plugin's config module (`config` in its
 * `rpp.json`) default-exports the result, which projects call in `rpp.config.ts`.
 */
export function definePluginConfig<Options>(
  name: string,
  hooks: PluginConfigOptions<Options> = {},
): (options: Options, access?: Access) => PluginEntry {
  return (options, access) => {
    const problems = hooks.validate?.(options) ?? [];
    if (problems.length > 0) {
      throw new Error(`invalid options for plugin \`${name}\`:\n  - ${problems.join("\n  - ")}`);
    }
    const normalized = hooks.normalize ? hooks.normalize(options) : options;
    return { ...access, plugin: name, options: normalized };
  };
}
