import { definePlugin } from "#rpp";

export default definePlugin({
  processors: {
    upper: {
      files: ["*.txt"],
      run(_ctx, file) {
        file.text = file.text.toUpperCase();
      },
    },
  },
});
