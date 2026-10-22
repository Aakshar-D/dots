import { useQuery } from "@tanstack/react-query";
import { Globe } from "lucide-react";

import { commands } from "@/api/commands";
import { browserLine, screenshotName } from "@/lib/browser";

/// A call of the built-in browser in a transcript (browser spec section 7): one line that says
/// what it did, its result folded under it, and a screenshot it took shown as an image.
export function BrowserCall(props: {
  runId: string;
  tool: string;
  input: unknown;
  result: { output: string; isError: boolean } | null;
  /// The run is still going: a call without a result is running.
  active: boolean;
}) {
  const { result } = props;
  const shot = result && !result.isError ? screenshotName(result.output) : null;
  const image = useQuery({
    queryKey: ["browser-screenshot", props.runId, shot],
    queryFn: () => commands.browserScreenshot(props.runId, shot ?? ""),
    enabled: shot !== null,
    staleTime: Infinity,
  });
  const outcome =
    result === null ? (props.active ? "running" : "no result") : result.isError ? "failed" : "done";
  return (
    <div className="flex flex-col gap-1">
      <details className="rounded-md border px-2 py-1">
        <summary className="flex cursor-pointer items-center gap-2 text-xs">
          <Globe aria-hidden className="size-3.5 shrink-0" />
          <span className="min-w-0 flex-1 truncate">
            {browserLine(props.tool, props.input, result)}
          </span>
          <span className={result?.isError ? "text-destructive" : "text-muted-foreground"}>
            {outcome}
          </span>
        </summary>
        {result && (
          <pre
            className={`mt-1 max-h-80 overflow-auto text-xs whitespace-pre-wrap ${
              result.isError ? "text-destructive" : "text-muted-foreground"
            }`}
          >
            {result.output}
          </pre>
        )}
      </details>
      {image.data && (
        <img
          src={image.data}
          alt="Screenshot of the dot's browser"
          className="max-h-80 self-start rounded-md border"
        />
      )}
    </div>
  );
}
