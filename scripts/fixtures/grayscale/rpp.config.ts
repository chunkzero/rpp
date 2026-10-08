import { defineConfig, plugin } from "rpp:config";

export default defineConfig({
  pack: { name: "grayscale-fixture", description: "Release consumer", format: 34 },
  build: { source: "src", output: "dist" },
  plugins: [plugin("grayscale-wasm")],
});
