import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState, type ReactNode } from "react";

import { commands } from "@/api/commands";
import type { PageState, Stage } from "@/api/types";
import { ConfirmButton } from "@/components/ConfirmButton";
import { ErrorNote } from "@/components/ErrorNote";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { localDateTime } from "@/lib/format";
import { errorMessage } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";

const HALF_DONE =
  "GitHub already has a signing key. Ask whoever set it up for the public key, or remove both secrets on GitHub to start again.";

function stageText(stage: Stage): string {
  switch (stage.stage) {
    case "waiting":
      return "Waiting to start";
    case "testing":
      return "Testing";
    case "building":
      return "Building";
    case "signing":
      return "Signing";
    case "published":
      return "Published";
    case "failed":
      return `Failed at ${stage.step}`;
  }
}

const running = (stage: Stage) => !["published", "failed"].includes(stage.stage);

function Card(props: { title: string; children: ReactNode }) {
  return (
    <section className="flex max-w-3xl flex-col gap-3 rounded-lg border p-4" aria-label={props.title}>
      <h2 className="text-sm font-semibold">{props.title}</h2>
      {props.children}
    </section>
  );
}

/// The one-time setup (updates UI spec section 5.2). `keyPending`: the secrets are in already,
/// so only step 4 is left.
function SetupCard({ keyPending }: { keyPending: boolean }) {
  const qc = useQueryClient();
  const [password, setPassword] = useState("");
  const [again, setAgain] = useState("");
  const [made, setMade] = useState(keyPending);
  const [savedTo, setSavedTo] = useState<string | null>(null);
  const [ticked, setTicked] = useState(keyPending);
  const [secretsIn, setSecretsIn] = useState(keyPending);
  const differ = again !== "" && again !== password;
  const make = useMutation({
    mutationFn: () => commands.releaseMakeKey(password),
    onSuccess: () => setMade(true),
  });
  const save = useMutation({ mutationFn: commands.releaseSaveKey, onSuccess: setSavedTo });
  const put = useMutation({
    mutationFn: () => commands.releasePutSecrets(password),
    onSuccess: () => {
      setSecretsIn(true);
      invalidate(qc, keys.releasesPage);
    },
  });
  const pull = useMutation({ mutationFn: commands.releaseOpenKeyPull });
  const failed = make.error ?? save.error ?? put.error ?? pull.error;
  return (
    <Card title="Set up releases">
      <p className="text-sm">Releases aren't set up yet.</p>
      <ol className="flex list-decimal flex-col gap-3 pl-5 text-sm">
        <li className="flex flex-col gap-2">
          <span className="font-medium">Make the signing key {made && "✓"}</span>
          <Label htmlFor="release-password">Password</Label>
          <Input
            id="release-password"
            type="password"
            className="max-w-xs"
            value={password}
            disabled={made}
            onChange={(e) => setPassword(e.target.value)}
          />
          <Label htmlFor="release-password-again">Password again</Label>
          <Input
            id="release-password-again"
            type="password"
            className="max-w-xs"
            value={again}
            disabled={made}
            aria-invalid={differ || undefined}
            aria-describedby={differ ? "release-password-differ" : undefined}
            onChange={(e) => setAgain(e.target.value)}
          />
          {differ && (
            <p id="release-password-differ" className="text-xs text-destructive">
              The two passwords differ.
            </p>
          )}
          <Button
            size="sm"
            className="self-start"
            disabled={made || make.isPending || password === "" || again !== password}
            onClick={() => make.mutate()}
          >
            Make the signing key
          </Button>
        </li>
        <li className="flex flex-col gap-2">
          <span className="font-medium">Save a backup {savedTo && ticked && "✓"}</span>
          <p>
            If this key and its password are both lost, no installed Dots can ever take another
            update.
          </p>
          <Button
            size="sm"
            variant="outline"
            className="self-start"
            disabled={!made || save.isPending}
            onClick={() => save.mutate()}
          >
            Save key file…
          </Button>
          {savedTo && <p className="text-xs text-muted-foreground">Saved to {savedTo}</p>}
          <label className="flex items-center gap-2">
            <input
              type="checkbox"
              checked={ticked}
              disabled={!savedTo}
              onChange={(e) => setTicked(e.target.checked)}
            />
            I have saved the key file and its password somewhere other than this PC
          </label>
        </li>
        <li className="flex flex-col gap-2">
          <span className="font-medium">Put it in GitHub's secrets {secretsIn && "✓"}</span>
          <ConfirmButton
            title="Put it in GitHub's secrets"
            description="Dots stores the key and its password as the repository's two signing secrets, then forgets them. Your backup is the only other copy."
            confirmLabel="Put it in GitHub's secrets"
            variant="default"
            disabled={!made || !savedTo || !ticked || secretsIn || put.isPending}
            onConfirm={() => put.mutate()}
          >
            Put it in GitHub's secrets
          </ConfirmButton>
        </li>
        <li className="flex flex-col gap-2">
          <span className="font-medium">Put the public key into Dots</span>
          <ConfirmButton
            title="Put the public key into Dots"
            description={`Dots opens a pull request, "Use the release signing key", that puts the public key into Dots.`}
            confirmLabel="Open the pull request"
            variant="default"
            disabled={!secretsIn || pull.isPending || !!pull.data}
            onConfirm={() => pull.mutate()}
          >
            Put the public key into Dots
          </ConfirmButton>
          {pull.data && (
            <a href={pull.data.url} target="_blank" rel="noreferrer" className="underline">
              Open pull request #{pull.data.number}
            </a>
          )}
          <p className="text-xs text-muted-foreground">
            Releases are set up once that pull request is merged.
          </p>
        </li>
      </ol>
      {failed && (
        <p role="alert" className="text-sm text-destructive">
          {errorMessage(failed)}
        </p>
      )}
    </Card>
  );
}

function ReadyCard({ page }: { page: PageState }) {
  const qc = useQueryClient();
  const [version, setVersion] = useState(page.suggested);
  const rc = page.next_rcs[version] ?? 1;
  const publish = useMutation({
    mutationFn: () => commands.releasePublishTrial(version),
    onSettled: () => invalidate(qc, keys.releasesPage),
  });
  const withdraw = useMutation({
    mutationFn: (v: string) => commands.releaseWithdraw(v),
    onSettled: () => invalidate(qc, keys.releasesPage),
  });
  const liveVersion = page.live?.tag.replace(/^v/, "");
  return (
    <Card title="Publish">
      <p className="text-sm">
        Live for everyone:{" "}
        {page.live && liveVersion
          ? `${liveVersion} · ${localDateTime(page.live.published_at)}`
          : "nothing published yet"}
      </p>
      {page.live && liveVersion && (
        <ConfirmButton
          title="Withdraw a release"
          description={`No more Dots will take ${liveVersion}. Dots already on it will be offered to go back to the version before it.`}
          confirmLabel="Withdraw"
          disabled={withdraw.isPending}
          onConfirm={() => withdraw.mutate(liveVersion)}
        >
          Withdraw {liveVersion}…
        </ConfirmButton>
      )}
      <div className="text-sm">
        Changes on master since it:
        <ul className="ml-5 list-disc">
          {page.changes.map((c) => (
            <li key={c.number}>
              {c.title} (#{c.number})
            </li>
          ))}
        </ul>
      </div>
      <div className="flex items-end gap-2">
        <div className="flex flex-col gap-1">
          <Label htmlFor="release-version">Version</Label>
          <Input
            id="release-version"
            className="w-28"
            value={version}
            onChange={(e) => setVersion(e.target.value)}
          />
        </div>
        <ConfirmButton
          title="Publish trial"
          description={`Publish ${version}-rc.${rc} from master as it is now? It goes only to publishers' Dots.`}
          confirmLabel="Publish trial"
          disabled={publish.isPending}
          onConfirm={() => publish.mutate()}
        >
          Publish trial
        </ConfirmButton>
      </div>
      {(publish.error ?? withdraw.error) && (
        <p role="alert" className="text-sm text-destructive">
          {errorMessage(publish.error ?? withdraw.error)}
        </p>
      )}
    </Card>
  );
}

function TrialCard({ trial }: { trial: NonNullable<PageState["trial"]> }) {
  const qc = useQueryClient();
  const [notes, setNotes] = useState(trial.notes);
  useEffect(() => setNotes(trial.notes), [trial.notes]);
  const done = () => invalidate(qc, keys.releasesPage);
  const saveNotes = useMutation({
    mutationFn: () => commands.releaseSaveNotes(trial.version, notes),
    onSettled: done,
  });
  const promote = useMutation({
    mutationFn: () => commands.releasePromote(trial.version),
    onSettled: done,
  });
  const drop = useMutation({
    mutationFn: () => commands.releaseDropTrial(trial.version),
    onSettled: done,
  });
  const n = trial.newer_on_master;
  const failed = saveNotes.error ?? promote.error ?? drop.error;
  return (
    <Card title="Trial">
      <p className="text-sm">
        Trial {trial.tag} · {stageText(trial.stage)}
        {trial.run && (
          <>
            {" · "}
            <a href={trial.run.url} target="_blank" rel="noreferrer" className="underline">
              View on GitHub
            </a>
          </>
        )}
      </p>
      {n > 0 && (
        <p className="text-sm text-muted-foreground">
          master has {n} {n === 1 ? "change" : "changes"} newer than this trial; publish a new
          trial to include them
        </p>
      )}
      <Label htmlFor="release-notes">What's new</Label>
      <Textarea
        id="release-notes"
        rows={6}
        value={notes}
        onChange={(e) => setNotes(e.target.value)}
      />
      <div className="flex flex-wrap gap-2">
        <ConfirmButton
          title="Save notes"
          description={`Dots commits these notes on release/${trial.version}; they become what's new in ${trial.version}.`}
          confirmLabel="Save notes"
          disabled={saveNotes.isPending}
          onConfirm={() => saveNotes.mutate()}
        >
          Save notes
        </ConfirmButton>
        <ConfirmButton
          title="Promote to everyone"
          description={`Every Dots will be offered ${trial.version} within six hours. It is exactly the code of ${trial.version}-rc.${trial.rc}.`}
          confirmLabel="Promote"
          variant="default"
          // Only a trial whose tests passed and that was published (final review I2).
          disabled={promote.isPending || trial.stage.stage !== "published"}
          onConfirm={() => promote.mutate()}
        >
          Promote {trial.version} to everyone
        </ConfirmButton>
        <ConfirmButton
          title="Drop trial"
          description={`Drop ${trial.tag}? Publishers' Dots stop being offered it.`}
          confirmLabel="Drop trial"
          disabled={drop.isPending}
          onConfirm={() => drop.mutate()}
        >
          Drop trial
        </ConfirmButton>
      </div>
      {failed && (
        <p role="alert" className="text-sm text-destructive">
          {errorMessage(failed)}
        </p>
      )}
    </Card>
  );
}

function PromotionCard({ promotion }: { promotion: NonNullable<PageState["promotion"]> }) {
  const qc = useQueryClient();
  const retry = useMutation({
    mutationFn: () => commands.releaseRetry(promotion.tag),
    onSettled: () => invalidate(qc, keys.releasesPage),
  });
  return (
    <Card title="Promotion">
      <p className="text-sm">
        Promoting {promotion.version} · {stageText(promotion.stage)}
        {promotion.run && (
          <>
            {" · "}
            <a href={promotion.run.url} target="_blank" rel="noreferrer" className="underline">
              View on GitHub
            </a>
          </>
        )}
      </p>
      {promotion.stage.stage === "failed" && (
        <ConfirmButton
          title="Retry the build"
          description={`GitHub runs the build of ${promotion.tag} again. If it passes, every Dots is offered ${promotion.version}.`}
          confirmLabel="Retry"
          variant="default"
          disabled={retry.isPending}
          onConfirm={() => retry.mutate()}
        >
          Retry
        </ConfirmButton>
      )}
      {retry.error && (
        <p role="alert" className="text-sm text-destructive">
          {errorMessage(retry.error)}
        </p>
      )}
    </Card>
  );
}

/// The Releases page (updates UI spec section 5), for publishers only.
export function ReleasesScreen() {
  const publisher = useQuery({ queryKey: keys.publisher, queryFn: commands.publisherStatus }).data;
  const isPublisher = publisher?.state.state === "publisher";
  const page = useQuery({
    queryKey: keys.releasesPage,
    queryFn: commands.releasesPage,
    enabled: isPublisher,
    // The build is read at most every 10 s while it runs.
    refetchInterval: (q) => {
      const d = q.state.data;
      const busy = (d?.trial && running(d.trial.stage)) || (d?.promotion && running(d.promotion.stage));
      return busy ? 10_000 : false;
    },
  });
  return (
    <div className="flex flex-col gap-4 overflow-y-auto p-6">
      <h1 className="text-lg font-semibold">Releases</h1>
      {publisher && !isPublisher && (
        <p className="text-sm text-muted-foreground">
          Dots is not set up to publish. Choose a GitHub token in Settings › Updates.
        </p>
      )}
      {page.isError && <ErrorNote error={page.error} />}
      {page.data?.setup.state === "half_done" && (
        <p role="alert" className="max-w-3xl text-sm">
          {HALF_DONE}
        </p>
      )}
      {(page.data?.setup.state === "needed" || page.data?.setup.state === "key_pending") && (
        <SetupCard keyPending={page.data.setup.state === "key_pending"} />
      )}
      {page.data?.setup.state === "done" && (
        <>
          <ReadyCard key={page.data.suggested} page={page.data} />
          {page.data.trial && <TrialCard trial={page.data.trial} />}
          {page.data.promotion && <PromotionCard promotion={page.data.promotion} />}
        </>
      )}
    </div>
  );
}
