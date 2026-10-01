import { definePlugin } from "#rpp";

import type { Options } from "./config.ts";

// A pure processor: it re-encodes JSON and `.mcmeta` files compactly, or pretty-printed
// when `pretty` is set. Assigning `file.text` is what marks the file as modified.
export default definePlugin<Options>({
  processors: {
    minify: {
      // Resource packs use `.json` for models, blockstates and language files, and `.mcmeta`
      // for pack and texture animation metadata. Both are JSON.
      files: ["**/*.json", "**/*.mcmeta"],
      // Runs late, so plugins that emit or rewrite JSON see whitespace before it is collapsed.
      priority: 100,
      run(ctx, file) {
        let decoded: unknown;
        try {
          decoded = JSON.parse(file.text);
        } catch {
          // Some packs ship intentionally quirky JSON; pass it through unchanged.
          return;
        }
        file.text = JSON.stringify(decoded, null, ctx.options.pretty === true ? 2 : undefined);
      },
    },
  },
});
