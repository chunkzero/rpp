import { components, definePlugin } from "rpp";

// Heavy per-file work (here, PNG decode/encode) lives in the component; TypeScript only
// routes bytes. Processor component calls get a fresh guest instance per file, so the guest
// may keep whatever state it likes.
export default definePlugin({
  processors: {
    grayscale: {
      files: "assets/*/textures/**/*.png",
      // Hash-renaming runs in the generator phase and sees these final bytes.
      priority: 5,
      run(_ctx, file) {
        const { exports } = components.load("grayscale");
        file.bytes = exports.grayscale(file.bytes);
      },
    },
  },
});
