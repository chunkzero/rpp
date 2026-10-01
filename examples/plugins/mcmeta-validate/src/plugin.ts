import { definePlugin } from "#rpp";

import { validateAnimation, validatePack } from "./rules.ts";

const parse = (path: string, text: string): unknown => {
  try {
    return JSON.parse(text);
  } catch (error) {
    throw new Error(`${path}: not valid JSON: ${error}`);
  }
};

// Validation spans the whole project (`pack_format` must match `rpp.config.ts`), so it belongs
// in the generator phase. It emits no files: it either passes or fails the build.
export default definePlugin({
  generate(ctx) {
    // Source copies are read so validation does not depend on what processors did to the
    // output. Each read is tracked, so the build re-validates only when an input changes.
    const manifest = ctx.readSourceText("pack.mcmeta");
    if (manifest === undefined) throw new Error("pack.mcmeta is missing from the pack source");
    const problems = validatePack(parse("pack.mcmeta", manifest), ctx.pack.format);

    // Enumerate the source tree so dropping or renaming processors cannot hide a file.
    const animations = ctx.sourceFiles("**/*.png.mcmeta");
    for (const path of animations) {
      problems.push(...validateAnimation(path, parse(path, ctx.readSourceText(path)!)));
    }

    if (problems.length > 0) {
      throw new Error(`mcmeta validation failed:\n  - ${problems.join("\n  - ")}`);
    }

    console.info(`validated pack.mcmeta and ${animations.length} animation file(s); all OK`);
  },
});
