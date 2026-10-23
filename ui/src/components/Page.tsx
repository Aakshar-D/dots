import type { ReactNode } from "react";

/// A screen: its heading (18px), optional actions on the right, then the content, 24px in and
/// 16px between sections (deep-teal spec section 4).
export function Page(props: { title: string; actions?: ReactNode; children?: ReactNode }) {
  return (
    <div className="flex flex-col gap-4 p-6">
      <header className="flex items-center gap-3">
        <h1 className="flex-1 truncate text-lg font-semibold">{props.title}</h1>
        {props.actions}
      </header>
      {props.children}
    </div>
  );
}
