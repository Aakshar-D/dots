import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowLeft, FolderOpen, Network } from "lucide-react";
import { useDeferredValue, useId, useState } from "react";
import { Link, useParams } from "react-router";

import { commands } from "@/api/commands";
import type { GraphStatus, NodeDetail, ViewNode } from "@/api/types";
import { EmptyState } from "@/components/EmptyState";
import { ErrorNote, LoadErrors } from "@/components/ErrorNote";
import { Page } from "@/components/Page";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { relativeTime } from "@/lib/format";
import { notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";
import { GraphCanvas, communityColor } from "@/screens/graph/GraphCanvas";

const n = (v: number) => v.toLocaleString();

/// "Built 5 min ago · 412 files · 5,210 nodes · 14,877 links", or how far a build is.
function statusLine(s: GraphStatus): string {
  const parts: string[] = [];
  if (s.state === "building") parts.push(`Building, ${s.progress ?? 0}%`);
  if (s.built_at !== null) {
    parts.push(`Built ${relativeTime(s.built_at)}`);
    parts.push(
      s.files === 1 ? "1 file" : `${n(s.files)} files`,
      `${n(s.nodes)} nodes`,
      `${n(s.edges)} edges`,
    );
    if (s.skipped > 0) parts.push(`${n(s.skipped)} skipped`);
    if (s.parse_errors === 1) parts.push("1 file could not be parsed");
    if (s.parse_errors > 1) parts.push(`${n(s.parse_errors)} files could not be parsed`);
    if (s.truncated) parts.push("stopped at the file cap; part of the project is left out");
  }
  return parts.join(" · ");
}

/// A node as a line: its label, kind and where it is.
function NodeButton(props: { node: ViewNode; onSelect: (id: string) => void }) {
  const { node } = props;
  const where = node.file !== null ? `${node.file}${node.line > 0 ? `:${node.line}` : ""}` : "";
  return (
    <button
      type="button"
      className="flex w-full min-w-0 items-center gap-2 rounded px-2 py-1 text-left text-sm hover:bg-muted"
      aria-label={[node.label, node.kind, where].filter((part) => part !== "").join(", ")}
      onClick={() => props.onSelect(node.id)}
    >
      <span
        aria-hidden
        className="size-2 shrink-0 rounded-full"
        style={{ backgroundColor: communityColor(node.community) }}
      />
      <span className="truncate font-medium" title={node.label}>
        {node.label}
      </span>
      <span className="shrink-0 text-xs text-muted-foreground">{node.kind}</span>
      <span className="truncate text-xs text-muted-foreground" title={where}>
        {where}
      </span>
    </button>
  );
}

/// How many nodes the list shows before Show more, and how many each press adds.
const LIST_STEP = 50;

/// The nodes on the canvas as a list the keyboard and screen readers can reach.
function NodeList(props: { nodes: ViewNode[]; onSelect: (id: string) => void }) {
  const [shown, setShown] = useState(LIST_STEP);
  const rest = props.nodes.length - shown;
  return (
    <section aria-label="Nodes in view">
      <h2 className="text-xs font-medium text-muted-foreground">
        Nodes in view ({props.nodes.length})
      </h2>
      <ul>
        {props.nodes.slice(0, shown).map((m) => (
          <li key={m.id}>
            <NodeButton node={m} onSelect={props.onSelect} />
          </li>
        ))}
      </ul>
      {rest > 0 && (
        <Button size="sm" variant="ghost" onClick={() => setShown(shown + LIST_STEP)}>
          Show {Math.min(rest, LIST_STEP)} more
        </Button>
      )}
    </section>
  );
}

/// The selected node: where it is, its signature, its neighbours by link, Show in folder.
function NodePanel(props: {
  dotId: string;
  detail: NodeDetail;
  onSelect: (id: string) => void;
}) {
  const { detail } = props;
  const node = detail.node;
  const reveal = useMutation({
    mutationFn: () => commands.graphReveal(props.dotId, node.id),
    onError: notifyError,
  });
  return (
    <aside aria-label="Selected node" className="flex flex-col gap-3">
      <div>
        <h2 className="text-base font-semibold break-all">{node.label}</h2>
        <p className="text-xs text-muted-foreground">
          {node.kind}
          {detail.community_name !== "" && ` · cluster ${detail.community_name}`}
        </p>
        {node.file !== null && (
          <p className="font-mono text-xs break-all">
            {node.file}
            {node.line > 0 && `:${node.line}`}
          </p>
        )}
        {detail.signature !== "" && (
          <pre
            tabIndex={0}
            aria-label="Signature"
            className="mt-1 overflow-x-auto rounded bg-muted p-2 text-xs"
          >
            {detail.signature}
          </pre>
        )}
      </div>
      {node.file !== null && (
        <Button
          size="sm"
          variant="outline"
          className="self-start"
          disabled={reveal.isPending}
          onClick={() => reveal.mutate()}
        >
          <FolderOpen aria-hidden /> Show in folder
        </Button>
      )}
      {detail.groups.map((g) => (
        <section key={g.label} aria-label={g.label}>
          <h3 className="text-xs font-medium text-muted-foreground">
            {g.label} ({g.nodes.length + g.more})
          </h3>
          <ul>
            {g.nodes.map((m) => (
              <li key={m.id}>
                <NodeButton node={m} onSelect={props.onSelect} />
              </li>
            ))}
          </ul>
          {g.more > 0 && <p className="px-2 text-xs text-muted-foreground">and {g.more} more</p>}
        </section>
      ))}
    </aside>
  );
}

/// The Graph screen of a dot (project graph spec section 10): its project's code graph,
/// coloured by community, with a search box, a side panel for the selected node, and the
/// build's state with Rebuild and Export.
export default function GraphScreen() {
  const { dotId = "" } = useParams();
  const qc = useQueryClient();
  const searchId = useId();
  const [community, setCommunity] = useState<number | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const deferred = useDeferredValue(query.trim());

  const dot = useQuery({ queryKey: keys.dot(dotId), queryFn: () => commands.getDot(dotId) });
  const on = dot.data?.use_graph ?? false;
  const status = useQuery({
    queryKey: keys.graphStatus(dotId),
    queryFn: () => commands.graphStatus(dotId),
    enabled: on,
  });
  const built = status.data?.built_at != null;
  const view = useQuery({
    queryKey: keys.graphView(dotId, community),
    queryFn: () => commands.graphView(dotId, community),
    enabled: on && built,
  });
  const detail = useQuery({
    queryKey: keys.graphNode(dotId, selected ?? ""),
    queryFn: () => commands.graphNode(dotId, selected ?? ""),
    enabled: on && built && selected !== null && !selected.startsWith("community:"),
  });
  const hits = useQuery({
    queryKey: keys.graphSearch(dotId, deferred),
    queryFn: () => commands.graphSearch(dotId, deferred),
    enabled: on && built && deferred.length > 0,
  });
  const rebuild = useMutation({
    mutationFn: () => commands.graphRebuild(dotId),
    onSuccess: () => invalidate(qc, ["graph", "status"]),
    onError: notifyError,
  });
  const exported = useMutation({
    mutationFn: () => commands.graphExport(dotId),
    onError: notifyError,
  });

  const select = (id: string) => {
    if (id.startsWith("community:")) {
      setCommunity(Number(id.slice("community:".length)));
      setSelected(null);
    } else {
      setSelected(id);
    }
  };

  const title = dot.data ? `${dot.data.name} · Graph` : "Graph";
  const back = (
    <Button asChild size="icon" variant="ghost" aria-label="Back to the dot">
      <Link to={`/home/${dotId}`}>
        <ArrowLeft aria-hidden />
      </Link>
    </Button>
  );
  const s = status.data;
  const building = s?.state === "building";
  const actions = on && s && (
    <div className="flex items-center gap-2">
      <Button
        size="sm"
        variant="outline"
        disabled={building || rebuild.isPending}
        onClick={() => rebuild.mutate()}
      >
        {built ? "Rebuild" : "Build graph"}
      </Button>
      {built && (
        <Button
          size="sm"
          variant="outline"
          disabled={exported.isPending}
          onClick={() => exported.mutate()}
        >
          Export for graphify
        </Button>
      )}
    </div>
  );

  if (dot.isError || status.isError) {
    return (
      <Page title={title} leading={back}>
        <LoadErrors queries={[dot, status]} />
      </Page>
    );
  }
  if (dot.data && !on) {
    return (
      <Page title={title} leading={back}>
        <EmptyState icon={Network}>
          The project graph is off for this dot. Turn on Use project graph in{" "}
          <Link className="underline" to={`/dots/${dotId}`}>
            its settings
          </Link>
          .
        </EmptyState>
      </Page>
    );
  }
  return (
    <Page title={title} leading={back} actions={actions || undefined}>
      {s && (
        <p role="status" className="text-sm text-muted-foreground">
          {statusLine(s)}
          {exported.data && ` · Exported to ${exported.data}`}
        </p>
      )}
      {s?.state === "failed" && s.error !== null && (
        <ErrorNote error={new Error(`The last build failed: ${s.error}`)} />
      )}
      {s && !built && !building && s.state !== "failed" && (
        <EmptyState icon={Network}>
          {s.git
            ? "No graph yet. Build graph reads the project's code once; runs keep it up to date."
            : "This folder is not a git repository, so Dots builds its graph only when you ask. " +
              "Build graph reads it once; after that, runs keep it up to date."}
        </EmptyState>
      )}
      {built && (
        <p className="text-xs text-muted-foreground">
          Export for graphify writes graphify-out/graph.json in the project. Add graphify-out/ to
          .gitignore so it is not committed.
        </p>
      )}
      {s && !built && building && (
        <EmptyState icon={Network}>Building the graph, {s.progress ?? 0}% done.</EmptyState>
      )}
      {built && (
        <div className="grid min-h-[480px] grid-cols-1 gap-4 lg:grid-cols-[1fr_320px]">
          <div className="flex min-h-[480px] flex-col gap-2">
            {view.data?.collapsed && (
              <p className="text-xs text-muted-foreground">
                A large graph: each circle is a cluster. Pick one to see its nodes.
              </p>
            )}
            {community !== null && (
              <Button
                size="sm"
                variant="ghost"
                className="self-start"
                onClick={() => {
                  setCommunity(null);
                  setSelected(null);
                }}
              >
                <ArrowLeft aria-hidden /> All clusters
              </Button>
            )}
            {view.isError && <ErrorNote error={view.error} />}
            {view.data && (
              <div className="flex-1">
                <GraphCanvas
                  nodes={view.data.nodes}
                  edges={view.data.edges}
                  selected={selected}
                  onSelect={select}
                />
              </div>
            )}
          </div>
          <div className="flex min-w-0 flex-col gap-3">
            <div className="flex flex-col gap-1">
              <label htmlFor={searchId} className="text-xs font-medium">
                Search the graph
              </label>
              <Input
                id={searchId}
                type="search"
                placeholder="A name or a path"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
              />
            </div>
            {deferred.length > 0 && hits.data && (
              <section aria-label="Search results">
                {hits.data.length === 0 ? (
                  <p className="px-2 text-sm text-muted-foreground">Nothing matches.</p>
                ) : (
                  <ul>
                    {hits.data.map((m) => (
                      <li key={m.id}>
                        <NodeButton node={m} onSelect={select} />
                      </li>
                    ))}
                  </ul>
                )}
              </section>
            )}
            {detail.isError && <ErrorNote error={detail.error} />}
            {detail.data && <NodePanel dotId={dotId} detail={detail.data} onSelect={select} />}
            {view.data && !view.data.collapsed && (
              <NodeList key={community ?? "all"} nodes={view.data.nodes} onSelect={select} />
            )}
            {view.data?.collapsed && community === null && (
              <section aria-label="Clusters">
                <h2 className="text-xs font-medium text-muted-foreground">Clusters</h2>
                <ul>
                  {view.data.nodes.map((m) => (
                    <li key={m.id}>
                      <NodeButton node={m} onSelect={select} />
                    </li>
                  ))}
                </ul>
              </section>
            )}
          </div>
        </div>
      )}
    </Page>
  );
}
