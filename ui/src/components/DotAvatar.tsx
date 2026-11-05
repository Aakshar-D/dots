import { cn } from "cn";
import type { CSSProperties } from "react";

import { avatarFor, type AvatarShape } from "@/lib/dot-avatar";

const SIZE = { sm: "size-4", md: "size-6" } as const;

/// Points of a regular polygon in a 24 by 24 box, one corner straight up.
function polygon(sides: number, radius = 11, inner?: number): string {
  const count = inner === undefined ? sides : sides * 2;
  return Array.from({ length: count }, (_, i) => {
    const r = inner !== undefined && i % 2 === 1 ? inner : radius;
    const angle = (i / count) * Math.PI * 2 - Math.PI / 2;
    return `${(12 + r * Math.cos(angle)).toFixed(2)},${(12 + r * Math.sin(angle)).toFixed(2)}`;
  }).join(" ");
}

/// A rounded square between a square and a circle (a superellipse), as a polygon.
function squircle(): string {
  return Array.from({ length: 48 }, (_, i) => {
    const t = (i / 48) * Math.PI * 2;
    const c = Math.cos(t);
    const s = Math.sin(t);
    const x = 12 + 11 * Math.sign(c) * Math.abs(c) ** 0.45;
    const y = 12 + 11 * Math.sign(s) * Math.abs(s) ** 0.45;
    return `${x.toFixed(2)},${y.toFixed(2)}`;
  }).join(" ");
}

const POLYGONS: Partial<Record<AvatarShape, string>> = {
  hexagon: polygon(6),
  triangle: "12,1.5 22.5,21 1.5,21",
  diamond: "12,1 23,12 12,23 1,12",
  pentagon: polygon(5),
  star: polygon(5, 11.5, 5),
  squircle: squircle(),
};

/// A dot's generated picture, so dots are easy to tell apart at a glance. It is decoration: the
/// dot's name is always shown beside it. Its colours come from `.dot-avatar` in index.css, which
/// has a set for each theme.
export function DotAvatar(props: { seed: string; size?: keyof typeof SIZE; className?: string }) {
  const { shape, hue, mark } = avatarFor(props.seed);
  const poly = POLYGONS[shape];
  return (
    <svg
      aria-hidden
      focusable="false"
      viewBox="0 0 24 24"
      data-avatar={shape}
      data-mark={mark}
      className={cn("dot-avatar shrink-0", SIZE[props.size ?? "sm"], props.className)}
      style={{ "--avatar-hue": hue } as CSSProperties}
    >
      {shape === "circle" && <circle cx="12" cy="12" r="11" className="dot-avatar-body" />}
      {shape === "rounded-square" && (
        <rect x="1.5" y="1.5" width="21" height="21" rx="6" className="dot-avatar-body" />
      )}
      {poly && <polygon points={poly} strokeLinejoin="round" className="dot-avatar-body" />}
      {mark === "dot" && <circle cx="12" cy="13" r="3" className="dot-avatar-mark" />}
      {mark === "ring" && (
        <circle cx="12" cy="13" r="4" fill="none" strokeWidth="2" className="dot-avatar-mark-line" />
      )}
      {mark === "stripe" && (
        <rect x="7" y="11.5" width="10" height="3" rx="1.5" className="dot-avatar-mark" />
      )}
    </svg>
  );
}
