// A line diff of two texts, for the card of instructions a dot proposed (conversational chat
// spec section 4).

export type DiffLine = { kind: "same" | "added" | "removed"; text: string };

/// The lines of `after` against those of `before`: a longest common subsequence of lines is
/// kept, what only `before` has is removed and what only `after` has is added, a removal
/// before the addition that takes its place.
export function lineDiff(before: string, after: string): DiffLine[] {
  const a = before === "" ? [] : before.split("\n");
  const b = after === "" ? [] : after.split("\n");
  // keep[i][j]: the longest common subsequence of a[i..] and b[j..].
  const keep = Array.from({ length: a.length + 1 }, () => new Array<number>(b.length + 1).fill(0));
  for (let i = a.length - 1; i >= 0; i--) {
    for (let j = b.length - 1; j >= 0; j--) {
      keep[i][j] = a[i] === b[j] ? keep[i + 1][j + 1] + 1 : Math.max(keep[i + 1][j], keep[i][j + 1]);
    }
  }
  const out: DiffLine[] = [];
  let i = 0;
  let j = 0;
  while (i < a.length && j < b.length) {
    if (a[i] === b[j]) {
      out.push({ kind: "same", text: a[i] });
      i++;
      j++;
    } else if (keep[i + 1][j] >= keep[i][j + 1]) {
      out.push({ kind: "removed", text: a[i++] });
    } else {
      out.push({ kind: "added", text: b[j++] });
    }
  }
  while (i < a.length) out.push({ kind: "removed", text: a[i++] });
  while (j < b.length) out.push({ kind: "added", text: b[j++] });
  return out;
}
