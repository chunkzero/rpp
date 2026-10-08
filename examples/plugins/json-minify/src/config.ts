import { definePluginConfig } from "rpp:config";

export interface Options {
  /** Re-indent instead of minifying; handy for diffing a built pack. */
  pretty?: boolean;
}

export default definePluginConfig<Options>("json-minify");
