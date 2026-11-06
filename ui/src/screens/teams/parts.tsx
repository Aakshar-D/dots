import type { ReactNode } from "react";

/// A titled part of the team page, named by its heading, with its actions on the right.
export function TeamSection(props: {
  id: string;
  title: string;
  actions?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section aria-labelledby={props.id} className="flex flex-col gap-2">
      <div className="flex items-center gap-3">
        <h2 id={props.id} className="flex-1 font-semibold">
          {props.title}
        </h2>
        {props.actions}
      </div>
      {props.children}
    </section>
  );
}
