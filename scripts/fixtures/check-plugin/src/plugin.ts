import { definePlugin, type Plugin } from "#rpp";

const plugin: Plugin = definePlugin({
  processors: {
    upper: {
      files: ["*.txt"],
      run(_ctx, file) {
        file.text = file.text.toUpperCase();
      },
    },
  },
});

export default plugin;
