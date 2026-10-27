import { useEffect, useLayoutEffect, useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import { connectFrame, SANDBOX, type Bridge } from "@/lib/app-bridge";
import { useResolvedTheme } from "@/lib/theme";

/// An app's screen in its sandboxed frame (app library spec section 5), filling its box. The
/// page gets its bridge on its first load. A page that loads again has navigated away from what
/// Dots served: it is cut off, and the person can load it afresh. `changes` counts the changes
/// to the app's data; each new count tells the page (`dots.onChange`).
export function AppFrame(props: {
  src: string;
  title: string;
  tools: readonly string[];
  call: (tool: string, input: unknown) => Promise<unknown>;
  changes?: number;
}) {
  const frame = useRef<HTMLIFrameElement>(null);
  const bridge = useRef<Bridge | null>(null);
  const loads = useRef(0);
  const [left, setLeft] = useState(false);
  const [fresh, setFresh] = useState(0);
  const theme = useResolvedTheme();
  // What the bridge reads at each call, without a new bridge on each render.
  const latest = useRef({ tools: props.tools, call: props.call, theme });
  latest.current = { tools: props.tools, call: props.call, theme };
  useEffect(() => bridge.current?.theme(theme), [theme]);
  useEffect(() => {
    if ((props.changes ?? 0) > 0) bridge.current?.changed();
  }, [props.changes]);
  // Another page (a new version, another preview) is a new frame with its own first load.
  useLayoutEffect(() => {
    loads.current = 0;
  }, [props.src]);
  useEffect(
    () => () => {
      bridge.current?.close();
      bridge.current = null;
    },
    [],
  );
  const onLoad = () => {
    loads.current += 1;
    bridge.current?.close();
    bridge.current = null;
    const page = frame.current?.contentWindow;
    if (loads.current > 1 || !page) {
      setLeft(true);
      return;
    }
    bridge.current = connectFrame(page, {
      get tools() {
        return latest.current.tools;
      },
      call: (tool, input) => latest.current.call(tool, input),
    });
    bridge.current.theme(latest.current.theme);
  };
  if (left) {
    return (
      <div
        role="alert"
        className="m-4 flex flex-col items-start gap-2 rounded-md border border-needs-border bg-needs-surface p-3"
      >
        <p>The app's screen left its page, so Dots stopped answering it.</p>
        <Button
          size="sm"
          onClick={() => {
            loads.current = 0;
            setLeft(false);
            setFresh((n) => n + 1);
          }}
        >
          Load the app again
        </Button>
      </div>
    );
  }
  return (
    <iframe
      key={`${fresh}:${props.src}`}
      ref={frame}
      title={props.title}
      src={props.src}
      sandbox={SANDBOX}
      onLoad={onLoad}
      className="min-h-0 w-full flex-1 border-0 bg-background"
    />
  );
}
