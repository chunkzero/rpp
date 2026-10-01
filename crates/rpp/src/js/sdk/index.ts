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

/** A module found by one of the plugin's `discover` patterns. */
export interface DiscoveredModule<M = Record<string, unknown>> {
  /** Source-relative path, e.g. `shop/window/main/window.ts`. */
  readonly path: string;
  /** The segment matched by the pattern's first whole `*` segment, if any. */
  readonly namespace?: string;
  /** The module namespace object. */
  readonly module: M;
}

/** Available to every handler, including processors. */
export interface Context<Options = unknown> {
  /** The plugin id. */
  readonly plugin: string;
  /** The plugin's configured options. */
  readonly options: Options;
  readonly pack: Pack;
}

/**
 * Available to the generator, `onStart` and `onFinish`. Processors do not get `discovered`,
 * because their cached results are not invalidated when discovered modules change; top-level
 * side effects of discovered modules are not tracked for processors either.
 */
export interface HookContext<Options = unknown> extends Context<Options> {
  /** Modules matched by the `discover` pattern `name`, sorted by path. Throws if undeclared. */
  discovered<M = Record<string, unknown>>(name: string): readonly DiscoveredModule<M>[];
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
export interface GeneratorContext<Options = unknown> extends HookContext<Options> {
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
  onStart?(ctx: HookContext<Options>): void | Promise<void>;
  onFinish?(ctx: HookContext<Options>, stats: BuildStats): void | Promise<void>;
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
  basename(p: string): string {
    return baseName(p);
  },
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

/** Wire form of a component value type. */
type TypeDesc =
  | string
  | { list: TypeDesc }
  | { record: [string, TypeDesc][] }
  | { tuple: TypeDesc[] }
  | { variant: [string, TypeDesc | null][] }
  | { enum: string[] }
  | { option: TypeDesc }
  | { result: { ok: TypeDesc | null; err: TypeDesc | null } }
  | { flags: string[] }
  | { unsupported: string };

interface FunctionDesc {
  path: string;
  params: [string, TypeDesc][];
  results: TypeDesc[];
}

/** Lower camelCase per kebab-case word, matching Rust's `heck`. */
function camelCase(name: string): string {
  return name
    .split("-")
    .filter((word) => word !== "")
    .map((word, i) => {
      const lower = word.toLowerCase();
      return i === 0 ? lower : lower.charAt(0).toUpperCase() + lower.slice(1);
    })
    .join("");
}

/** A component call that failed inside the component and returned `err`. */
export class ComponentError extends Error {
  readonly component: string;
  readonly export: string;
  readonly payload: unknown;

  constructor(component: string, exportName: string, payload: unknown) {
    super(
      `component \`${component}\` export \`${exportName}\` failed: ${describePayload(payload)}`,
    );
    this.name = "ComponentError";
    this.component = component;
    this.export = exportName;
    this.payload = payload;
  }
}

/** The component trapped. The handle is unusable afterwards. */
export class ComponentTrapError extends Error {
  readonly component: string;
  readonly export: string;

  constructor(component: string, exportName: string, message: string) {
    super(`component \`${component}\` export \`${exportName}\` trapped: ${message}`);
    this.component = component;
    this.name = "ComponentTrapError";
    this.export = exportName;
  }
}

/** The component exceeded its execution deadline. The handle is unusable afterwards. */
export class ComponentTimeoutError extends Error {
  readonly component: string;
  readonly export: string;

  constructor(component: string, exportName: string, message: string) {
    super(`component \`${component}\` export \`${exportName}\` timed out: ${message}`);
    this.component = component;
    this.name = "ComponentTimeoutError";
    this.export = exportName;
  }
}

const describePayload = (payload: unknown): string => {
  if (typeof payload === "string") return payload;
  if (isObject(payload) && typeof payload["message"] === "string") return payload["message"];
  try {
    return JSON.stringify(payload, (_key, value) =>
      typeof value === "bigint" ? value.toString() : value,
    );
  } catch {
    return String(payload);
  }
};

const isObject = (value: unknown): value is Record<string, any> =>
  typeof value === "object" && value !== null;

const isOption = (ty: TypeDesc): boolean => isObject(ty) && "option" in ty;

/** Byte lists of one call, concatenated into the call's single attachment. */
class ByteSink {
  readonly chunks: Uint8Array[] = [];
  length = 0;

  push(chunk: Uint8Array): [number, number] {
    const range: [number, number] = [this.length, chunk.length];
    this.chunks.push(chunk);
    this.length += chunk.length;
    return range;
  }

  finish(): Uint8Array | undefined {
    if (this.chunks.length === 0) return undefined;
    const out = new Uint8Array(this.length);
    let offset = 0;
    for (const chunk of this.chunks) {
      out.set(chunk, offset);
      offset += chunk.length;
    }
    return out;
  }
}

const float = (value: unknown): number | string => {
  const n = Number(value);
  if (Number.isNaN(n)) return "NaN";
  if (n === Infinity) return "Infinity";
  if (n === -Infinity) return "-Infinity";
  if (Object.is(n, -0)) return "-0";
  return n;
};

const mismatch = (ty: TypeDesc, value: unknown): TypeError =>
  new TypeError(`expected ${JSON.stringify(ty)}, got ${describePayload(value)}`);

const field = (value: unknown, key: string, ty: TypeDesc): any => {
  if (!isObject(value)) throw mismatch(ty, value);
  return value[key];
};

/** Convert a jco value to its wire form. */
function encode(ty: TypeDesc, value: any, sink: ByteSink): unknown {
  if (typeof ty === "string") {
    switch (ty) {
      case "s64":
      case "u64":
        return BigInt(value).toString();
      case "f32":
      case "f64":
      case "float32":
      case "float64":
        return float(value);
      default:
        return value;
    }
  }
  if ("list" in ty) {
    if (ty.list === "u8") {
      if (!(value instanceof Uint8Array)) throw mismatch(ty, value);
      return sink.push(value);
    }
    if (!Array.isArray(value)) throw mismatch(ty, value);
    return value.map((item) => encode(ty.list, item, sink));
  }
  if ("record" in ty) {
    return Object.fromEntries(
      ty.record.map(([name, t]) => [name, encode(t, field(value, camelCase(name), ty), sink)]),
    );
  }
  if ("tuple" in ty) {
    if (!Array.isArray(value)) throw mismatch(ty, value);
    return ty.tuple.map((t, i) => encode(t, value[i], sink));
  }
  if ("variant" in ty) {
    const tag = field(value, "tag", ty);
    const found = ty.variant.find(([name]) => name === tag);
    if (found === undefined) throw mismatch(ty, value);
    return { tag, val: found[1] === null ? null : encode(found[1], value.val, sink) };
  }
  if ("enum" in ty) {
    if (!ty.enum.includes(value)) throw mismatch(ty, value);
    return value;
  }
  if ("option" in ty) {
    if (isOption(ty.option)) {
      const tag = field(value, "tag", ty);
      if (tag === "none") return [];
      if (tag === "some") return [encode(ty.option, value.val, sink)];
      throw mismatch(ty, value);
    }
    return value === undefined || value === null ? [] : [encode(ty.option, value, sink)];
  }
  if ("result" in ty) {
    const tag = field(value, "tag", ty);
    if (tag !== "ok" && tag !== "err") throw mismatch(ty, value);
    const payload = tag === "ok" ? ty.result.ok : ty.result.err;
    return { [tag]: payload === null ? null : encode(payload, value.val, sink) };
  }
  if ("flags" in ty) {
    if (!isObject(value)) throw mismatch(ty, value);
    return ty.flags.filter((name) => value[camelCase(name)] === true);
  }
  throw new TypeError(`unsupported component type \`${ty.unsupported}\``);
}

/** Convert a wire value to its jco form. */
function decode(ty: TypeDesc, wire: any, bytes: Uint8Array): unknown {
  if (typeof ty === "string") {
    switch (ty) {
      case "s64":
      case "u64":
        return BigInt(wire);
      case "f32":
      case "f64":
      case "float32":
      case "float64":
        return typeof wire === "string" ? Number(wire) : wire;
      default:
        return wire;
    }
  }
  if ("list" in ty) {
    if (ty.list === "u8") return bytes.slice(wire[0], wire[0] + wire[1]);
    return wire.map((item: unknown) => decode(ty.list, item, bytes));
  }
  if ("record" in ty) {
    return Object.fromEntries(
      ty.record.map(([name, t]) => [camelCase(name), decode(t, wire[name], bytes)]),
    );
  }
  if ("tuple" in ty) return ty.tuple.map((t, i) => decode(t, wire[i], bytes));
  if ("variant" in ty) {
    const payload = ty.variant.find(([name]) => name === wire.tag)?.[1] ?? null;
    return payload === null
      ? { tag: wire.tag }
      : { tag: wire.tag, val: decode(payload, wire.val, bytes) };
  }
  if ("option" in ty) {
    const some = wire.length > 0;
    if (isOption(ty.option)) {
      return some ? { tag: "some", val: decode(ty.option, wire[0], bytes) } : { tag: "none" };
    }
    return some ? decode(ty.option, wire[0], bytes) : undefined;
  }
  if ("result" in ty) {
    const tag = "ok" in wire ? "ok" : "err";
    const payload = tag === "ok" ? ty.result.ok : ty.result.err;
    return payload === null ? { tag } : { tag, val: decode(payload, wire[tag], bytes) };
  }
  if ("flags" in ty) {
    return Object.fromEntries(ty.flags.map((name) => [camelCase(name), wire.includes(name)]));
  }
  if ("enum" in ty) return wire;
  throw new TypeError(`unsupported component type \`${ty.unsupported}\``);
}

/** A loaded component. Every `load` creates an independent instance, released when the
 * current handler finishes. */
export interface Component<Exports = ComponentExports> {
  readonly exports: Exports;
}

/** Untyped exports: functions by camelCase name, interfaces as namespaces of functions. */
export interface ComponentExports {
  [name: string]: any;
}

/** Maps component names to their types. Augmented by `.rpp/generated/*.d.ts`. */
export interface ComponentMap {}

type Invoke = (desc: FunctionDesc, args: unknown[]) => unknown;

const exportsFactories = new Map<string, (invoke: Invoke) => ComponentExports>();

const buildExports = (functions: FunctionDesc[]) => (invoke: Invoke) => {
  const exports: ComponentExports = Object.create(null);
  for (const desc of functions) {
    const hash = desc.path.indexOf("#");
    const fn = (...args: unknown[]) => invoke(desc, args);
    if (hash < 0) {
      exports[camelCase(desc.path)] = fn;
      continue;
    }
    const iface = desc.path.slice(0, hash);
    const namespace = camelCase(iface.slice(iface.lastIndexOf("/") + 1).split("@")[0] ?? "");
    if (!Object.hasOwn(exports, namespace)) exports[namespace] = Object.create(null);
    exports[namespace][camelCase(desc.path.slice(hash + 1))] = fn;
  }
  return exports;
};

function loadComponent(name: string): Component {
  const reply = __rpp.call("component.load", { name }).value;
  if (reply.failure !== undefined)
    throw new ComponentTimeoutError(name, "load", reply.failure.message);
  const handle: string = reply.handle;
  let factory = exportsFactories.get(name);
  if (factory === undefined) {
    factory = buildExports(reply.functions);
    exportsFactories.set(name, factory);
  }
  let poisoned = false;

  const invoke: Invoke = (desc, args) => {
    if (poisoned)
      throw new Error(`component \`${name}\` handle is unusable after a trap or timeout`);
    if (args.length !== desc.params.length) {
      throw new TypeError(
        `component \`${name}\` export \`${desc.path}\` expects ${desc.params.length} argument(s), got ${args.length}`,
      );
    }
    const sink = new ByteSink();
    const wireArgs = desc.params.map(([, ty], i) => encode(ty, args[i], sink));
    const result = __rpp.call(
      "component.call",
      { handle, path: desc.path, args: wireArgs },
      sink.finish(),
    );
    const { failure, results } = result.value;
    if (failure !== undefined) {
      poisoned = true;
      const Failure = failure.kind === "timeout" ? ComponentTimeoutError : ComponentTrapError;
      throw new Failure(name, desc.path, failure.message);
    }
    const bytes = result.bytes ?? new Uint8Array(0);
    const values = desc.results.map((ty, i) => decode(ty, results[i], bytes));
    if (values.length === 0) return undefined;
    const [first] = desc.results;
    if (values.length === 1 && isObject(first) && "result" in first) {
      const outcome = values[0] as { tag: "ok" | "err"; val?: unknown };
      if (outcome.tag === "err") throw new ComponentError(name, desc.path, outcome.val);
      return outcome.val;
    }
    return values.length === 1 ? values[0] : values;
  };

  return { exports: factory(invoke) };
}

function load<N extends keyof ComponentMap>(name: N): Component<ComponentMap[N]>;
function load(name: string): Component;
function load(name: string): Component {
  return loadComponent(name);
}

/** WASM components declared in the plugin manifest. Only usable inside handlers. */
export const components: { load: typeof load } = { load };
