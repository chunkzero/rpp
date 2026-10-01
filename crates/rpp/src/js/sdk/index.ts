// The rpp plugin SDK, imported as `#rpp`. Embedded in rpp and written to
// `.rpp/sdk/index.ts` by `rpp codegen`, so it always matches the running rpp.

declare const __rpp: {
  call(
    name: string,
    value: unknown,
    bytes?: Uint8Array,
  ): { value: any; bytes: Uint8Array | undefined };
};

const encoder = new TextEncoder();

const toBytes = (data: Uint8Array | string): Uint8Array =>
  typeof data === "string" ? encoder.encode(data) : data;

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

export const toml = {
  parse(text: string): unknown {
    return __rpp.call("toml.parse", { text }).value;
  },
  stringify(value: unknown): string {
    return __rpp.call("toml.stringify", { value }).value;
  },
};

const digest = (algorithm: string, data: Uint8Array | string) =>
  __rpp.call("hash", { algorithm }, toBytes(data)).value;

export const hash = {
  xxh3: (data: Uint8Array | string): string => digest("xxh3", data),
  sha256: (data: Uint8Array | string): string => digest("sha256", data),
  md5: (data: Uint8Array | string): string => digest("md5", data),
  crc32: (data: Uint8Array | string): number => digest("crc32", data),
};

const baseName = (p: string): string => p.slice(p.lastIndexOf("/") + 1);

const dotIndex = (base: string): number => {
  const dot = base.lastIndexOf(".");
  return dot > 0 ? dot : -1;
};

export const path = {
  join(...parts: string[]): string {
    const segments: string[] = [];
    for (const segment of parts.join("/").split("/")) {
      if (segment === "..") segments.pop();
      else if (segment !== "" && segment !== ".") segments.push(segment);
    }
    return segments.join("/");
  },
  dirname(p: string): string {
    const slash = p.lastIndexOf("/");
    return slash < 0 ? "" : p.slice(0, slash);
  },
  basename: baseName,
  ext(p: string): string {
    const base = baseName(p);
    const dot = dotIndex(base);
    return dot < 0 ? "" : base.slice(dot + 1);
  },
  withExt(p: string, ext: string): string {
    const slash = p.lastIndexOf("/");
    const base = p.slice(slash + 1);
    const dot = dotIndex(base);
    const stem = dot < 0 ? base : base.slice(0, dot);
    const suffix = ext.replace(/^\.+/, "");
    return p.slice(0, slash + 1) + (suffix === "" ? stem : `${stem}.${suffix}`);
  },
  match(pattern: string, p: string): boolean {
    return __rpp.call("glob.match", { pattern, path: p }).value;
  },
};

/** Run a program. Requires `trusted` security with the program in `permissions.process`;
 * only available in the generator and hooks. */
export const process = {
  run(request: ProcessRequest): ProcessResult {
    const reply = __rpp.call(
      "process.run",
      {
        program: request.program,
        args: request.args ?? [],
        cwd: request.cwd,
        env: request.env ?? {},
        timeout_ms: request.timeoutMs,
      },
      request.stdin === undefined ? undefined : toBytes(request.stdin),
    );
    return {
      status: reply.value.status,
      stdout: reply.bytes ?? new Uint8Array(0),
      stderr: reply.value.stderr,
    };
  },
};
