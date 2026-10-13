import type { ReactNode } from "react";

/// A screen: its heading, optional actions on the right, then the content.
export function Page(props: { title: string; actions?: ReactNode; children?: ReactNode }) {
  return (
    <div className="flex flex-col gap-4 p-5">
      <header className="flex items-center gap-3">
        <h1 className="flex-1 truncate text-lg font-semibold">{props.title}</h1>
        {props.actions}
      </header>
      {props.children}
    </div>
  );
}
