import { definePluginConfig } from "#rpp/config";

export interface Options {
  /** Namespace of the generated models and translations. */
  namespace: string;
}

/** An item definition: the default export of each `items/*.ts` file. */
export interface Item {
  name: string;
  /** A namespaced texture resource location, such as `minecraft:custom/gem`. */
  texture: string;
}

export default definePluginConfig<Options>("example-catalog", {
  validate: (options) =>
    /^[a-z0-9_-]+$/.test(options.namespace)
      ? []
      : ["namespace must contain lowercase letters, digits, underscores or hyphens"],
});
