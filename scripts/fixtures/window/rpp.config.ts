import { defineConfig } from "#rpp/config";
import window from "#plugins/window";

export default defineConfig({
  pack: { name: "window-fixture", description: "Release consumer", format: 88 },
  build: { source: "src", output: "dist" },
  plugins: [
    window({
      namespace: "release",
      kotlin: { package: "dev.chunkzero.release", output: "generated" },
    }),
  ],
});
