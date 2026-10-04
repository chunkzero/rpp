const isObject = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

const isPositiveInteger = (value: unknown): boolean => Number.isInteger(value) && Number(value) > 0;

const isIndex = (value: unknown): boolean => value === 0 || isPositiveInteger(value);

/**
 * Problems with one `*.png.mcmeta` animation file; empty when valid. Only the structurally
 * important fields are checked, see https://minecraft.wiki/w/Resource_pack#Animation.
 */
export function validateAnimation(path: string, data: unknown): string[] {
  if (!isObject(data)) return [`${path}: must be a JSON object`];
  const animation = data["animation"];
  if (!isObject(animation)) return [`${path}: missing \`animation\` object`];

  const problems: string[] = [];
  const { frametime, interpolate, frames } = animation;
  if (frametime !== undefined && !isPositiveInteger(frametime)) {
    problems.push(`${path}: animation.frametime must be a positive integer`);
  }
  if (interpolate !== undefined && typeof interpolate !== "boolean") {
    problems.push(`${path}: animation.interpolate must be a boolean`);
  }
  if (frames === undefined) return problems;
  if (!Array.isArray(frames)) {
    problems.push(`${path}: animation.frames must be an array`);
    return problems;
  }
  frames.forEach((frame: unknown, i) => {
    const where = `${path}: animation.frames[${i + 1}]`;
    if (!isObject(frame)) {
      if (!isIndex(frame)) problems.push(`${where} must be a non-negative integer`);
      return;
    }
    if (!isIndex(frame["index"])) problems.push(`${where}.index must be a non-negative integer`);
    if (frame["time"] !== undefined && !isPositiveInteger(frame["time"])) {
      problems.push(`${where}.time must be a positive integer`);
    }
  });
  return problems;
}
