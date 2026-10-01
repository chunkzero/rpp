import { definePluginConfig } from "#rpp/config";

export interface Options {
  /**
   * Globs of output files to fingerprint. Renaming vanilla textures would break the fixed
   * names models and blockstates reference, so the default covers only `custom/` assets.
   */
  files?: string[];
}

export default definePluginConfig<Options>("hash-rename", {
  normalize: (options) => ({ files: options.files ?? ["assets/*/textures/custom/**/*.png"] }),
});
