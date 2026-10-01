// The rpp plugin SDK, imported as `#rpp`. Embedded in rpp and written to
// `.rpp/sdk/index.ts` by `rpp codegen`, so it always matches the running rpp.

/** Pack metadata from the project configuration. */
export interface Pack {
  readonly name: string;
  readonly description?: string;
  readonly format?: number;
}

/** Available to every handler. */
export interface Context<Options = unknown> {
  /** The plugin id. */
  readonly plugin: string;
  /** The plugin's configured options. */
  readonly options: Options;
  readonly pack: Pack;
}

/** A file in the processor phase. Assigning `path`, `bytes` or `text` modifies it. */
export interface File {
  /** Relative, forward-slash output path. */
  path: string;
  bytes: Uint8Array;
  /** `bytes` decoded (and encoded on assignment) as UTF-8. */
  text: string;
  /** Exclude the file from output; later processors do not run on it. */
  drop(): void;
}

/** Available to the generator. Every read is tracked for incremental rebuilds. */
export interface GeneratorContext<Options = unknown> extends Context<Options> {
  /** Processed output paths, optionally filtered by a glob. */
  files(glob?: string): string[];
  /** Raw source paths, optionally filtered by a glob. */
  sourceFiles(glob?: string): string[];
  /** A processed output file, or `undefined` if absent. */
  read(path: string): Uint8Array | undefined;
  readText(path: string): string | undefined;
  /** A raw source file, or `undefined` if absent. */
  readSource(path: string): Uint8Array | undefined;
  readSourceText(path: string): string | undefined;
  /** Add or replace an output file. */
  emit(path: string, contents: Uint8Array | string): void;
  /** Remove an output file. */
  remove(path: string): void;
  /** Write a file under a configured named output root outside the pack. */
  emitOutput(root: string, path: string, contents: Uint8Array | string): void;
}

export interface BuildStats {
  readonly processed: number;
  readonly cached: number;
  readonly generated: number;
  readonly dropped: number;
}

export interface Processor<Options = unknown> {
  /** Globs selecting the files this processor runs on. */
  files: string | readonly string[];
  /** Lower runs first. Defaults to 0. */
  priority?: number;
  run(ctx: Context<Options>, file: File): void | Promise<void>;
}

export interface Plugin<Options = unknown> {
  processors?: Record<string, Processor<Options>>;
  generate?(ctx: GeneratorContext<Options>): void | Promise<void>;
  onStart?(ctx: Context<Options>): void | Promise<void>;
  onFinish?(ctx: Context<Options>, stats: BuildStats): void | Promise<void>;
}

/** Declare a plugin. The entry module must export the result as its default export. */
export function definePlugin<Options = unknown>(plugin: Plugin<Options>): Plugin<Options> {
  return plugin;
}

export interface ProcessRequest {
  program: string;
  args?: readonly string[];
  /** Working directory, relative to the project root. */
  cwd?: string;
  env?: Readonly<Record<string, string>>;
  stdin?: Uint8Array | string;
  timeoutMs?: number;
}

export interface ProcessResult {
  status: number;
  stdout: Uint8Array;
  stderr: string;
}

export declare const toml: {
  parse(text: string): unknown;
  stringify(value: unknown): string;
};

export declare const hash: {
  xxh3(data: Uint8Array | string): string;
  sha256(data: Uint8Array | string): string;
  md5(data: Uint8Array | string): string;
  crc32(data: Uint8Array | string): number;
};

export declare const path: {
  join(...parts: string[]): string;
  dirname(path: string): string;
  basename(path: string): string;
  /** The extension without the dot, or `""`. */
  ext(path: string): string;
  withExt(path: string, ext: string): string;
  /** Whether `path` matches the glob `pattern`, using rpp's glob rules. */
  match(pattern: string, path: string): boolean;
};

/** Run a program. Requires `trusted` security with the program in `permissions.process`;
 * only available in the generator and hooks. */
export declare const process: {
  run(request: ProcessRequest): ProcessResult;
};
