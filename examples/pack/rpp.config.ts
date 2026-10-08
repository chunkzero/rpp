import { defineConfig, plugin } from "rpp:config";
import catalog from "plugin:example-catalog";
import hashRename from "plugin:hash-rename";
import jsonMinify from "plugin:json-minify";

export default defineConfig({
  pack: {
    name: "rpp-example-pack",
    description: "A tiny but complete Minecraft resource pack, built end-to-end by rpp.",
    format: 34,
  },
  build: {
    source: "src",
    output: "dist",
    workers: 0, // 0 = use all available cores
    limits: { memoryLimitMb: 64, executionDeadlineSeconds: 5 },
    squash: {
      enabled: true,
      engine: "builtin",
      json: true, // minify .json/.mcmeta in the output
      png: "fast", // oxipng "fast" preset
      zip: true, // also write dist/rpp-example-pack.zip
    },
  },
  // Ordered list. Order is the tie-break for processors that share a priority.
  plugins: [
    // 1. Compact every JSON / .mcmeta file.
    jsonMinify({ pretty: false }),
    // 2. Validate texture animation metadata; fail the build on anything malformed.
    plugin("mcmeta-validate"),
    // 3. Fingerprint author-owned custom/ textures by content hash and emit rename_map.json.
    //    Scoped so vanilla texture references stay intact.
    hashRename({ files: ["assets/*/textures/custom/**/*.png"] }),
    // 4. Resolve renamed model textures and compile TypeScript item definitions into models,
    //    translations, and a catalog for tools outside the resource pack.
    catalog({ namespace: "rpp" }, { outputs: { catalog: "generated/catalog" } }),
  ],
});
