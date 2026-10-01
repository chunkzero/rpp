import { definePlugin } from "#rpp";

import type { Item, Options } from "./config.ts";

const json = (value: unknown, pretty = false): string =>
  JSON.stringify(value, null, pretty ? 2 : undefined);

// Item definitions are discovered modules, so they are authoring inputs that never reach the
// pack, and adding or editing one reruns the generator without any configuration change.
export default definePlugin<Options>({
  generate(ctx) {
    const { namespace } = ctx.options;
    const renames: Record<string, string> = JSON.parse(ctx.readText("rename_map.json")!);

    // Resolves a texture reference through hash-rename's output, which this generator sees.
    const texture = (reference: string): string => {
      const match = /^([^:]+):(.+)$/.exec(reference);
      if (match === null) throw new Error("texture must be a namespaced resource location");
      const original = `assets/${match[1]}/textures/${match[2]}.png`;
      const resolved = renames[original] ?? original;
      if (ctx.read(resolved) === undefined) throw new Error(`missing texture: ${reference}`);
      const parts = /^assets\/([^/]+)\/textures\/(.+)\.png$/.exec(resolved)!;
      return `${parts[1]}:${parts[2]}`;
    };

    // Existing models may be overwritten because this plugin declares `overrides`.
    for (const path of ctx.files("assets/*/models/**/*.json")) {
      const model = JSON.parse(ctx.readText(path)!);
      let changed = false;
      for (const [key, reference] of Object.entries<string>(model.textures ?? {})) {
        if (reference.startsWith("#")) continue;
        const resolved = texture(reference);
        if (resolved !== reference) {
          model.textures[key] = resolved;
          changed = true;
        }
      }
      if (changed) ctx.emit(path, json(model));
    }

    const translations: Record<string, string> = Object.create(null);
    const items: Record<string, unknown> = Object.create(null);
    for (const { path, module } of ctx.discovered<{ default: Item }>("items")) {
      const id = /^items\/([a-z0-9_-]+)\.ts$/.exec(path)?.[1];
      if (id === undefined) {
        throw new Error(`item filename must be a lowercase identifier: ${path}`);
      }
      const { name, texture: reference } = module.default;
      if (typeof name !== "string" || typeof reference !== "string") {
        throw new Error(`${path} must export name and texture strings`);
      }
      const resolved = texture(reference);
      const translation = `item.${namespace}.${id}`;
      translations[translation] = name;
      ctx.emit(
        `assets/${namespace}/models/item/${id}.json`,
        json({ parent: "minecraft:item/generated", textures: { layer0: resolved } }),
      );
      items[id] = { model: `${namespace}:item/${id}`, texture: resolved, translation };
    }
    ctx.emit(`assets/${namespace}/lang/en_us.json`, json(translations));
    ctx.emitOutput("catalog", "items.json", json({ items, pack: ctx.pack.name }, true));
  },
});
