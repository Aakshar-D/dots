import { useQuery } from "@tanstack/react-query";
import { cn } from "cn";
import { useEffect, useState } from "react";

import { commands } from "@/api/commands";
import type { EngineKind, ModelOption } from "@/api/types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { NativeSelect } from "@/components/ui/native-select";
import { isAuto } from "@/lib/auto-model";
import { errorMessage } from "@/lib/notify";
import { keys } from "@/lib/query";

/// The list without Auto: for a picker of one fixed model, such as Auto's own sizes.
export const withoutAuto = (options: ModelOption[]) => options.filter((o) => !isAuto(o.id));

/// The select's value for "Custom…". Not a model id any engine lists.
export const CUSTOM = "\u0000custom";

/// What the core takes as an endpoint (`check_endpoint_url`): http:// and a host.
const ENDPOINT = /^http:\/\/[^\s/]+/;
/// How long the endpoint must stay as it is before its models are asked for.
const ENDPOINT_WAIT_MS = 400;

/// The models a dot of `engine` can pick (model switcher spec section 2.1): the Claude list, or
/// what the local `endpoint` lists, asked for only once an endpoint is set.
export function useModels(engine: EngineKind, endpoint: string | null) {
  // Asked once the endpoint has stopped changing for a moment and looks like one, not for
  // every key typed into it.
  const typed = engine === "local" ? endpoint?.trim() || null : null;
  const [url, setUrl] = useState(typed);
  useEffect(() => {
    if (typed === url) return;
    const t = setTimeout(() => setUrl(typed), ENDPOINT_WAIT_MS);
    return () => clearTimeout(t);
  }, [typed, url]);
  const asked = engine === "local" ? url : null;
  const query = useQuery({
    queryKey: keys.models(engine, asked),
    queryFn: () => commands.listModels(engine, asked),
    enabled: engine === "claude" || (asked !== null && ENDPOINT.test(asked)),
    retry: false,
    staleTime: engine === "claude" ? Infinity : 30_000,
  });
  // While the endpoint is still being typed, the list of the one before is not offered.
  const settling = engine === "local" && typed !== url;
  const note =
    engine !== "local"
      ? undefined
      : typed === null || !ENDPOINT.test(typed)
        ? "Set the endpoint to list its models, or pick Custom… to type a model ID."
        : settling || query.isPending
          ? "Loading the endpoint's models…"
          : query.isError
            ? `Could not list the endpoint's models: ${errorMessage(query.error)} Pick Custom… to type a model ID.`
            : query.data.length === 0
              ? "The endpoint lists no models. Pick Custom… to type a model ID."
              : undefined;
  return { options: settling ? [] : (query.data ?? []), note };
}

/// A model picker (model switcher spec section 2.3): a select of `options` and Custom…, which
/// shows a text box for any model id. A value not in the list shows as Custom with its id; with
/// no list at all, it shows as itself.
/// `commit`: the custom id is applied with Enter or Use rather than on every key, for a picker
/// that saves at once. `compact`: no hint line, for a header.
export function ModelPicker(props: {
  id: string;
  value: string;
  onChange: (model: string) => void;
  options: ModelOption[];
  /// Said under the picker: why the list is empty or still loading.
  note?: string;
  commit?: boolean;
  compact?: boolean;
  disabled?: boolean;
  /// Names the select when no label points at it.
  ariaLabel?: string;
  title?: string;
  className?: string;
}) {
  const { value, options } = props;
  const listed = options.find((o) => o.id === value);
  // Custom when the value is set and not in the list, also after a listed one was picked and
  // the list changed. Picking Custom…, or typing in its box, holds Custom even while the box
  // is emptied.
  const [picked, setPicked] = useState(false);
  // With no list (loading, or none to be had) the value stands as its own option.
  const unlisted = options.length === 0 && value !== "";
  const custom = picked || (value !== "" && listed === undefined && !unlisted);
  const [draft, setDraft] = useState(value);
  const selected = custom ? CUSTOM : listed || unlisted ? value : "";
  function choose(next: string) {
    if (next === CUSTOM) {
      setPicked(true);
      setDraft(value);
      return;
    }
    setPicked(false);
    if (next !== value) props.onChange(next);
  }
  function use() {
    const id = draft.trim();
    if (id !== "" && id !== value) props.onChange(id);
  }
  const hint = !props.compact && !custom ? listed?.hint : undefined;
  return (
    <div className={cn("flex flex-col gap-1.5", props.className)}>
      <div className="flex flex-wrap items-center gap-2">
        <NativeSelect
          id={props.id}
          size={props.compact ? "sm" : "default"}
          className={props.compact ? "w-36" : "w-64"}
          aria-label={props.ariaLabel}
          title={props.title}
          disabled={props.disabled}
          value={selected}
          onChange={(e) => choose(e.target.value)}
        >
          {selected === "" && (
            <option value="" disabled>
              Choose a model
            </option>
          )}
          {unlisted && !custom && <option value={value}>{value}</option>}
          {options.map((o) => (
            <option key={o.id} value={o.id}>
              {o.label}
            </option>
          ))}
          <option value={CUSTOM}>Custom…</option>
        </NativeSelect>
        {custom &&
          (props.commit ? (
            <form
              className="flex items-center gap-1"
              onSubmit={(e) => {
                e.preventDefault();
                use();
              }}
            >
              <Input
                id={`${props.id}-custom`}
                aria-label="Custom model ID"
                placeholder="Model ID"
                className={cn("font-mono", props.compact ? "h-7 w-44" : "w-64")}
                disabled={props.disabled}
                value={draft}
                onChange={(e) => setDraft(e.target.value)}
              />
              <Button
                type="submit"
                size="sm"
                variant="outline"
                disabled={props.disabled || draft.trim() === "" || draft.trim() === value}
              >
                Use
              </Button>
            </form>
          ) : (
            <Input
              id={`${props.id}-custom`}
              aria-label="Custom model ID"
              placeholder="Model ID"
              className="w-64 font-mono"
              disabled={props.disabled}
              value={value}
              onChange={(e) => {
                setPicked(true);
                // A model id has no spaces around it.
                props.onChange(e.target.value.trim());
              }}
            />
          ))}
      </div>
      {hint && <p className="text-xs text-muted-foreground">{hint}</p>}
      {props.note && !props.compact && (
        <p className="text-xs text-muted-foreground">{props.note}</p>
      )}
    </div>
  );
}
