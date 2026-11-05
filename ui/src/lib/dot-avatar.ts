// A dot's generated avatar: a shape, a hue and a small inner mark, all taken from a seed so the
// same seed always gives the same picture. Nothing is stored and nothing is random.

export const AVATAR_SHAPES = [
  "circle",
  "rounded-square",
  "hexagon",
  "triangle",
  "diamond",
  "pentagon",
  "star",
  "squircle",
] as const;
export const AVATAR_MARKS = ["none", "dot", "ring", "stripe"] as const;

export type AvatarShape = (typeof AVATAR_SHAPES)[number];
export type AvatarMark = (typeof AVATAR_MARKS)[number];
export type Avatar = { shape: AvatarShape; hue: number; mark: AvatarMark };

/// FNV-1a over the seed's UTF-16 units, then a final mix so that the low bits are as good as
/// the high ones.
function hash(seed: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < seed.length; i++) {
    h ^= seed.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  h ^= h >>> 16;
  h = Math.imul(h, 0x85ebca6b);
  h ^= h >>> 13;
  h = Math.imul(h, 0xc2b2ae35);
  h ^= h >>> 16;
  return h >>> 0;
}

/// The avatar a seed gives. A dot's seed is its id, so renaming keeps it; before there is an id
/// (the new-dot form, the setup card) the seed is the name.
export function avatarFor(seed: string): Avatar {
  const h = hash(seed);
  return {
    shape: AVATAR_SHAPES[h % AVATAR_SHAPES.length]!,
    hue: Math.floor(h / 8) % 360,
    mark: AVATAR_MARKS[Math.floor(h / 2880) % AVATAR_MARKS.length]!,
  };
}
