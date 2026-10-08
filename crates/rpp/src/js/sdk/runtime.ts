/// <reference path="./bridge.d.ts" />
// The dispatcher bundled with every plugin as `rpp:internal/runtime`. Its exports are the functions rpp calls.

import type {
  BuildStats,
  Context,
  DiscoveredModule,
  File,
  GeneratorContext,
  HookContext,
  Pack,
  Plugin,
  Processor,
} from "rpp";

const encoder = new TextEncoder();
const decoder = new TextDecoder();
const empty = new Uint8Array(0);

const toBytes = (data: Uint8Array | string): Uint8Array =>
  typeof data === "string" ? encoder.encode(data) : data;

let discoveredModules: Record<string, readonly DiscoveredModule[]> = {};
let registered: Plugin<unknown> | undefined;
let context: Context<unknown> | undefined;
let hookContext: HookContext<unknown> | undefined;

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null;

/** Validate and record the entry module's default export. */
export function register(
  plugin: unknown,
  discovered: Record<string, readonly DiscoveredModule[]> = {},
): void {
  if (!isRecord(plugin)) throw new TypeError("the entry module must default-export a plugin");
  const { processors, generate, onStart, onFinish } = plugin;
  if (processors !== undefined) {
    if (!isRecord(processors)) throw new TypeError("`processors` must be an object");
    for (const [name, processor] of Object.entries(processors)) {
      if (!isRecord(processor)) throw new TypeError(`processor \`${name}\` must be an object`);
      const files = typeof processor.files === "string" ? [processor.files] : processor.files;
      if (
        !Array.isArray(files) ||
        files.length === 0 ||
        files.some((glob) => typeof glob !== "string" || glob === "")
      ) {
        throw new TypeError(`processor \`${name}\` needs a non-empty \`files\` glob or list`);
      }
      if (typeof processor.run !== "function") {
        throw new TypeError(`processor \`${name}\` needs a \`run\` function`);
      }
      const priority = processor.priority;
      if (priority !== undefined && !Number.isInteger(priority)) {
        throw new TypeError(`processor \`${name}\` priority must be an integer`);
      }
    }
  }
  for (const [name, handler] of Object.entries({ generate, onStart, onFinish })) {
    if (handler !== undefined && typeof handler !== "function") {
      throw new TypeError(`\`${name}\` must be a function`);
    }
  }
  registered = plugin as Plugin<unknown>;
  discoveredModules = discovered;
}

const processorTable = (): Record<string, Processor<unknown>> => registered?.processors ?? {};

export function describe(): unknown {
  return {
    processors: Object.entries(processorTable()).map(([name, processor]) => ({
      name,
      files: typeof processor.files === "string" ? [processor.files] : [...processor.files],
      priority: processor.priority ?? 0,
    })),
    generator: registered?.generate !== undefined,
    onStart: registered?.onStart !== undefined,
    onFinish: registered?.onFinish !== undefined,
  };
}

export function init(args: { plugin: string; options: unknown; pack: Pack }): null {
  const base = { plugin: args.plugin, options: args.options, pack: args.pack };
  context = {
    ...base,
    discovered: () => {
      throw new Error("ctx.discovered() is only available in generate/onStart/onFinish");
    },
  } as Context<unknown>;
  hookContext = {
    ...base,
    discovered: <M>(name: string) => {
      if (!Object.hasOwn(discoveredModules, name)) {
        throw new TypeError(`no \`discover\` pattern named \`${name}\``);
      }
      return discoveredModules[name] as readonly DiscoveredModule<M>[];
    },
  };
  return null;
}

class PluginFile implements File {
  #path: string;
  #bytes: Uint8Array;
  dropped = false;

  constructor(path: string, bytes: Uint8Array) {
    this.#path = path;
    this.#bytes = bytes;
  }

  get path(): string {
    return this.#path;
  }

  set path(value: string) {
    if (typeof value !== "string") throw new TypeError("file.path must be a string");
    this.#path = value;
  }

  get bytes(): Uint8Array {
    return this.#bytes;
  }

  set bytes(value: Uint8Array) {
    if (!(value instanceof Uint8Array)) throw new TypeError("file.bytes must be a Uint8Array");
    this.#bytes = value;
  }

  get text(): string {
    return decoder.decode(this.#bytes);
  }

  set text(value: string) {
    if (typeof value !== "string") throw new TypeError("file.text must be a string");
    this.#bytes = encoder.encode(value);
  }

  drop(): void {
    this.dropped = true;
  }
}

export async function process(
  args: { processor: string; path: string },
  bytes?: Uint8Array,
): Promise<Uint8Array> {
  const processor = processorTable()[args.processor];
  if (processor === undefined) throw new Error("no such processor");
  const file = new PluginFile(args.path, bytes ?? empty);
  await processor.run(context!, file);
  __rpp.call("file", { path: file.path, dropped: file.dropped });
  return file.bytes;
}

const list = (name: string, glob?: string): string[] => __rpp.call(name, { glob }).value;

const readFile = (name: string, path: string): Uint8Array | undefined => {
  const reply = __rpp.call(name, { path });
  return reply.value.found ? (reply.bytes ?? empty) : undefined;
};

const decode = (bytes: Uint8Array | undefined): string | undefined =>
  bytes === undefined ? undefined : decoder.decode(bytes);

export async function generate(): Promise<null> {
  const handler = registered?.generate;
  if (handler === undefined) return null;
  const ctx: GeneratorContext<unknown> = {
    ...hookContext!,
    files: (glob) => list("files", glob),
    sourceFiles: (glob) => list("source_files", glob),
    read: (path) => readFile("read", path),
    readText: (path) => decode(readFile("read", path)),
    readSource: (path) => readFile("read_source", path),
    readSourceText: (path) => decode(readFile("read_source", path)),
    emit(path, contents) {
      __rpp.call("emit", { path }, toBytes(contents));
    },
    remove(path) {
      __rpp.call("remove", { path });
    },
    emitOutput(root, path, contents) {
      __rpp.call("emit_output", { root, path }, toBytes(contents));
    },
  };
  await handler.call(registered, ctx);
  return null;
}

export async function onStart(): Promise<null> {
  await registered?.onStart?.call(registered, hookContext!);
  return null;
}

export async function onFinish(stats: BuildStats): Promise<null> {
  await registered?.onFinish?.call(registered, hookContext!, stats);
  return null;
}
