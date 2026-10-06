# WASM Components In Plugins

A TypeScript plugin may ship named WASIp2 components and call them with
`components.load` from the plugin SDK. Declare each component in the plugin's
`rpp.json`:

```json
{
  "name": "my-plugin",
  "version": "1.0.0",
  "entry": "src/plugin.ts",
  "components": { "compiler": "compiler.wasm" }
}
```

```ts
import { components, definePlugin } from "#rpp";

export default definePlugin({
  generate(ctx) {
    const { exports } = components.load("compiler");
    const files = exports.compile("assets/example/input.json");
    for (const file of files) ctx.emit(file.path, file.contents);
  },
});
```

`components.load` is only usable inside handlers (processors, `generate`, `onStart`,
`onFinish`). Each call creates an independent instance that is released when the handler
finishes.

`rpp codegen` (also run best-effort by `build` and `dev`) writes
`.rpp/generated/<name>.d.ts` for every built component, which augments `ComponentMap` so
`components.load("compiler")` is fully typed. Components that are not built yet are
skipped with a warning. `rpp check` type-checks the project against those declarations.

## Component Shape

Components may export ordinary WIT functions directly from their world:

```wit
package example:compiler;

world compiler {
    record output-file {
        path: string,
        contents: list<u8>,
    }

    export compile: func(input: string) -> result<list<output-file>, string>;
}
```

Exports are functions on `exports` by camelCase name; exports of a named interface are
grouped under the interface's camelCase name. Values are converted from the component type
signature:

| WIT            | TypeScript                                                              |
| -------------- | ----------------------------------------------------------------------- |
| `list<u8>`     | `Uint8Array`                                                            |
| `s64`, `u64`   | `bigint`                                                                |
| `record`       | object with camelCase fields                                            |
| `variant`      | `{ tag, val }` (no `val` for a payloadless case)                        |
| `enum`         | the case name as a string                                               |
| `flags`        | object of camelCase booleans                                            |
| `option<T>`    | `T` or `undefined`; nested options use `{ tag: "some" \| "none", val }` |
| `result<T, E>` | `T` on `ok`; throws `ComponentError` with the `err` payload             |

Type and range errors identify the component export and parameter that failed conversion.
A trap or exceeded deadline throws `ComponentTrapError` or `ComponentTimeoutError`, and the
handle is unusable afterwards.

## WASI And Trust

By default, components run without filesystem preopens, network access, passed
environment variables, or process execution. Standard Rust WASI imports such as
closed stdio, environment access with no variables, terminal probing, exit, and
random seed are linkable so ordinary Rust components instantiate. Without a
random grant, random interfaces receive deterministic streams so cached and cold
builds agree; `permissions.random: true` opts into host randomness and disables
replay for that plugin.

Calls from processor callbacks instantiate a fresh guest for each file. This
prevents guest globals or deterministic random-stream position from depending on
worker scheduling. Calls from a sequential generator or hook reuse the handle's
instance, which permits multi-call compiler workflows without cross-file state.

Component binaries participate in the plugin's cache key. RPP also caches
Wasmtime compilation by component content in memory and as user-wide precompiled
artifacts at `<cache>/wasmtime/<key>.cwasm`, shared across projects.
Per-instance memory and per-call time limits come from `build.wasm`
(`memoryLimitMb`, `executionDeadlineSeconds`) in `rpp.config.ts`.

[`examples/plugins/grayscale-wasm`](../examples/plugins/grayscale-wasm) is a
complete processor plugin with a Rust guest crate.

Project config may grant broader access only with `security: "trusted"`:

```ts
plugin("my-plugin", undefined, {
  security: "trusted",
  permissions: {
    read: ["data"],
    write: ["generated"],
    environment: ["MY_ENV_VAR"],
    process: ["my-tool"],
  },
});
```

`process` lets the plugin's generator and hooks start the listed programs with
`process.run` from the SDK.
