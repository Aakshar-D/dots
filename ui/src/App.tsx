import { useEffect, useState } from "react";

import { commands, type StartupError, type Status } from "./api/commands";

type View =
  | { kind: "loading" }
  | { kind: "fatal"; error: StartupError }
  | { kind: "ready"; status: Status }
  | { kind: "error"; message: string };

// Plan 3a placeholder that proves the command bridge; Plan 3b replaces it with the screens.
export function App() {
  const [view, setView] = useState<View>({ kind: "loading" });
  useEffect(() => {
    (async () => {
      const fatal = await commands.startupError();
      if (fatal) return setView({ kind: "fatal", error: fatal });
      setView({ kind: "ready", status: await commands.status() });
    })().catch((e: unknown) => setView({ kind: "error", message: String(e) }));
  }, []);

  switch (view.kind) {
    case "loading":
      return <p>Starting Dots…</p>;
    case "fatal":
      return (
        <main role="alert">
          <h1>Dots could not start</h1>
          <p>{view.error.message}</p>
          <p>Logs: {view.error.log_dir}</p>
        </main>
      );
    case "error":
      return <p role="alert">{view.message}</p>;
    case "ready":
      return (
        <main>
          <h1>Dots</h1>
          <pre>{JSON.stringify(view.status, null, 2)}</pre>
        </main>
      );
  }
}
