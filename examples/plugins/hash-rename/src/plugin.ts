import { definePlugin, hash, path } from "#rpp";

import type { Options } from "./config.ts";

// `logo.png` with content hash `deadbeef...` becomes `logo.deadbeef.png`.
const hashedPath = (file: string, bytes: Uint8Array): string => {
  const ext = path.ext(file);
  const stem = ext === "" ? path.basename(file) : path.basename(file).slice(0, -ext.length - 1);
  const name = `${stem}.${hash.xxh3(bytes).slice(0, 8)}${ext === "" ? "" : `.${ext}`}`;
  return path.join(path.dirname(file), name);
};

// Runs in the generator phase, so fingerprints cover the bytes that are actually written.
// `rename_map.json` maps original paths to hashed paths so references can be rewritten.
export default definePlugin<Required<Options>>({
  generate(ctx) {
    const renames: Record<string, string> = {};
    for (const pattern of ctx.options.files) {
      for (const file of ctx.files(pattern)) {
        if (file in renames) continue;
        const bytes = ctx.read(file)!;
        const hashed = hashedPath(file, bytes);
        ctx.emit(hashed, bytes);
        ctx.remove(file);
        renames[file] = hashed;
      }
    }
    ctx.emit("rename_map.json", JSON.stringify(renames, null, 2));
    const count = Object.keys(renames).length;
    console.info(`wrote rename_map.json with ${count} ${count === 1 ? "entry" : "entries"}`);
  },
});
