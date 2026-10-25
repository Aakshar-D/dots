import { act, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { JoinPreview, OrgSummary } from "@/api/types";
import { localDateTime } from "@/lib/format";
import { backend, later } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const [acme] = sample.orgs; // Acme Engineering, OpenBao, connected
const [newCode, keyChange] = sample.joinPreviews; // a new org; Acme's path and key change
// A join code's text: it holds no secret. The values below are made up for the tests.
const CODE = "eyJ2IjoxLCJvcmdfaWQiOiJhY21lLWVuZyIsIm5hbWUiOiJBY21lIEVuZ2luZWVyaW5nIn0";
const TOKEN = "hvs.ui-test-token-7f3a";
const PASSWORD = "ui-test-password-91c4";
const CLIENT_SECRET = "ui-test-client-secret-2b8e";
const AGE_KEY = "age1uitestpublickeyqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqs3290gq";
// An error whose words name the recipient case: the UI goes by the answer, not by words.
const RECIPIENT = "could not join Design: this PC's key is not a recipient of the file";

/// Answers `org_list` with what the test holds, like the core.
function orgs(initial: OrgSummary[]) {
  let current = initial;
  backend.on("org_list", () => current);
  return {
    set: (next: OrgSummary[]) => {
      current = next;
    },
  };
}

const section = () => screen.findByRole("group", { name: "Organisations" });
const row = async (name: string) =>
  within(await section()).findByRole("listitem", { name });
/// No secret the test typed shows anywhere but in its own field.
const noSecretShown = () => {
  for (const secret of [TOKEN, PASSWORD, CLIENT_SECRET]) {
    expect(document.body.textContent).not.toContain(secret);
  }
};

/// Opens Join (or Update on `org`), pastes the code and previews it.
async function preview(user: ReturnType<typeof renderApp>["user"], org?: string) {
  if (org) {
    await user.click(within(await row(org)).getByRole("button", { name: "Update" }));
  } else {
    await user.click(
      within(await section()).getByRole("button", { name: "Join an organisation" }),
    );
  }
  const dialog = await screen.findByRole("dialog");
  await user.click(within(dialog).getByLabelText("Join code"));
  await user.paste(CODE);
  await user.click(within(dialog).getByRole("button", { name: "Preview" }));
  return dialog;
}

describe("Settings, Organisations", () => {
  it("lists each org with its status in the tone of that status", async () => {
    const statuses = sample.orgs.map((org, i) => ({ ...org, id: `org-${i}`, name: `Org ${i}` }));
    orgs(statuses);
    renderApp("/settings");
    // Plain while it works; the attention colour while it works from its copy, has a policy it
    // refused or waits; the error colour once its values are gone.
    const tones: Record<OrgSummary["status"], string | null> = {
      connected: null,
      offline: "text-needs",
      policy_invalid: "text-needs",
      pending: "text-needs",
      revoked: "text-destructive",
      expired: "text-destructive",
    };
    for (const org of statuses) {
      const item = await row(org.name);
      const status = within(item).getByText(org.status_text);
      const tone = tones[org.status];
      for (const other of ["text-needs", "text-destructive"]) {
        if (other === tone) expect(status, org.status).toHaveClass(other);
        else expect(status, org.status).not.toHaveClass(other);
      }
    }
  });

  it("shows the source, the key, the times and, when asked, the policy", async () => {
    orgs([acme]);
    const { user } = renderApp("/settings");
    const item = await row("Acme Engineering");
    expect(item).toHaveTextContent("OpenBao at https://bao.acme.internal");
    expect(within(item).getByText(acme.key_fingerprint)).toBeVisible();
    expect(item).toHaveTextContent(`Last fetch${localDateTime(acme.last_fetch_at)}`);
    expect(item).toHaveTextContent(`Saved copy until${localDateTime(acme.copy_until)}`);
    // The policy is folded away until asked for.
    expect(within(item).queryByText(acme.policy_summary[1])).toBeNull();
    await user.click(within(item).getByRole("button", { name: "Show the policy" }));
    for (const line of acme.policy_summary) expect(within(item).getByText(line)).toBeVisible();
    await user.click(within(item).getByRole("button", { name: "Hide the policy" }));
    expect(within(item).queryByText(acme.policy_summary[1])).toBeNull();
  });

  it("says when no org is joined", async () => {
    renderApp("/settings");
    expect(
      await within(await section()).findByText("No organisation is joined on this PC."),
    ).toBeVisible();
  });

  it("refreshes an org and shows its new status", async () => {
    const list = orgs([acme]);
    const offline = sample.orgs[1];
    backend.on("org_refresh", () => {
      list.set([offline]);
      return offline;
    });
    const { user } = renderApp("/settings");
    const item = await row("Acme Engineering");
    await user.click(within(item).getByRole("button", { name: "Refresh" }));
    expect(await screen.findByText(offline.status_text)).toBeVisible();
    expect(backend.callsTo("org_refresh")).toEqual([{ orgId: "acme-eng" }]);
  });

  it("asks before it removes an org, in words that say what goes and what stays", async () => {
    const list = orgs([acme]);
    backend.on("org_remove", () => {
      list.set([]);
      return null;
    });
    const { user } = renderApp("/settings");
    await user.click(within(await row("Acme Engineering")).getByRole("button", { name: "Remove" }));
    let ask = await screen.findByRole("alertdialog", { name: "Remove Acme Engineering?" });
    expect(ask).toHaveTextContent(
      "The dots of Acme Engineering stay on this PC, but they can't run until you join it " +
        "again. The saved copy of its variables and your sign-in details for it are deleted.",
    );
    await user.click(within(ask).getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
    expect(backend.callsTo("org_remove")).toEqual([]);

    await user.click(within(await row("Acme Engineering")).getByRole("button", { name: "Remove" }));
    ask = await screen.findByRole("alertdialog", { name: "Remove Acme Engineering?" });
    await user.click(within(ask).getByRole("button", { name: "Remove" }));
    expect(
      await within(await section()).findByText("No organisation is joined on this PC."),
    ).toBeVisible();
    expect(backend.callsTo("org_remove")).toEqual([{ orgId: "acme-eng" }]);
  });

  it("joins with a token: the preview, then the token, which is cleared once sent", async () => {
    const list = orgs([]);
    backend.on("org_preview", () => newCode);
    const joined = later<OrgSummary>();
    backend.on("org_join", () => joined.answer);
    const { user } = renderApp("/settings");
    const dialog = await preview(user);

    expect(within(dialog).getByRole("heading", { name: "Join an organisation" })).toBeVisible();
    expect(dialog).toHaveTextContent("Acme Engineering");
    expect(dialog).toHaveTextContent("OpenBao at https://bao.acme.internal");
    expect(within(dialog).getByText(newCode.key_fingerprint)).toBeVisible();
    expect(backend.callsTo("org_preview")).toEqual([{ code: CODE }]);
    await user.click(within(dialog).getByRole("button", { name: "Continue" }));

    const token = within(dialog).getByLabelText("Token");
    expect(token).toHaveAttribute("type", "password");
    await user.type(token, TOKEN);
    noSecretShown();
    await user.click(within(dialog).getByRole("button", { name: "Join" }));
    // As `SourceCreds` serializes.
    expect(backend.callsTo("org_join")).toEqual([
      { code: CODE, creds: { kind: "token", token: TOKEN } },
    ]);
    expect(within(dialog).getByLabelText("Token")).toHaveValue("");

    list.set([acme]);
    act(() => joined.give(acme));
    expect(await screen.findByText("Joined Acme Engineering")).toBeVisible();
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(await row("Acme Engineering")).toBeVisible();
    noSecretShown();
  });

  it("joins with a username and password for a userpass mount", async () => {
    orgs([]);
    backend.on("org_preview", () => ({ ...newCode, auth: "userpass" }) satisfies JoinPreview);
    backend.on("org_join", () => acme);
    const { user } = renderApp("/settings");
    const dialog = await preview(user);
    await user.click(within(dialog).getByRole("button", { name: "Continue" }));
    expect(within(dialog).queryByLabelText("Token")).toBeNull();
    const join = within(dialog).getByRole("button", { name: "Join" });
    await user.type(within(dialog).getByLabelText("Username"), "ana");
    // Both are needed.
    expect(join).toBeDisabled();
    expect(within(dialog).getByLabelText("Password")).toHaveAttribute("type", "password");
    await user.type(within(dialog).getByLabelText("Password"), PASSWORD);
    noSecretShown();
    await user.click(join);
    expect(backend.callsTo("org_join")).toEqual([
      { code: CODE, creds: { kind: "login", username: "ana", password: PASSWORD } },
    ]);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("joins Infisical with a machine identity", async () => {
    orgs([]);
    backend.on("org_preview", () => ({
      ...newCode,
      source_kind: "infisical",
      address: "https://app.infisical.com",
      auth: null,
    }));
    backend.on("org_join", () => acme);
    const { user } = renderApp("/settings");
    const dialog = await preview(user);
    expect(dialog).toHaveTextContent("Infisical at https://app.infisical.com");
    await user.click(within(dialog).getByRole("button", { name: "Continue" }));
    await user.type(within(dialog).getByLabelText("Client id"), "cid-1");
    const secret = within(dialog).getByLabelText("Client secret");
    expect(secret).toHaveAttribute("type", "password");
    await user.type(secret, CLIENT_SECRET);
    noSecretShown();
    await user.click(within(dialog).getByRole("button", { name: "Join" }));
    expect(backend.callsTo("org_join")).toEqual([
      { code: CODE, creds: { kind: "machine", client_id: "cid-1", client_secret: CLIENT_SECRET } },
    ]);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("joins a SOPS org with this PC's key, and waits for the admin to add it", async () => {
    const list = orgs([]);
    const pending = sample.orgs.find((o) => o.status === "pending")!;
    backend.on("org_preview", () => ({
      ...newCode,
      org_id: "design",
      name: "Design",
      source_kind: "sops",
      address: pending.address,
      auth: null,
    }));
    backend.on("org_sops_key", () => AGE_KEY);
    // An error first, then pending twice, then joined.
    const answers: (() => OrgSummary)[] = [
      () => {
        throw { kind: "internal", message: RECIPIENT };
      },
      () => pending,
      () => pending,
      () => {
        list.set([{ ...acme, id: "design", name: "Design" }]);
        return { ...acme, id: "design", name: "Design" };
      },
    ];
    backend.on("org_join", () => answers.shift()!());
    const { user } = renderApp("/settings");
    const dialog = await preview(user);
    expect(dialog).toHaveTextContent(`SOPS file at ${pending.address}`);
    await user.click(within(dialog).getByRole("button", { name: "Continue" }));

    expect(await within(dialog).findByLabelText("This PC's key")).toHaveValue(AGE_KEY);
    expect(backend.callsTo("org_sops_key")).toEqual([{ orgId: "design" }]);
    expect(dialog).toHaveTextContent("Send this key to your admin, then press Join");
    await user.click(within(dialog).getByRole("button", { name: "Copy the key" }));
    expect(await navigator.clipboard.readText()).toBe(AGE_KEY);

    // An error is an error, whatever its words.
    await user.click(within(dialog).getByRole("button", { name: "Join" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(RECIPIENT);
    expect(within(dialog).queryByText("Waiting for your admin to add this PC's key")).toBeNull();
    // A pending answer is the wait.
    await user.click(within(dialog).getByRole("button", { name: "Join" }));
    expect(
      await within(dialog).findByText("Waiting for your admin to add this PC's key"),
    ).toBeVisible();
    expect(dialog).not.toHaveTextContent(pending.status_text);
    await user.click(within(dialog).getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(backend.callsTo("org_join")).toHaveLength(3));
    expect(within(dialog).getByText("Waiting for your admin to add this PC's key")).toBeVisible();
    await user.click(within(dialog).getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    // This PC's key is the core's to read: nothing is sent for it.
    expect(backend.callsTo("org_join")).toEqual([
      { code: CODE, creds: null },
      { code: CODE, creds: null },
      { code: CODE, creds: null },
      { code: CODE, creds: null },
    ]);
    expect(await row("Design")).toBeVisible();
  });

  it("updates an org: what changes, a key warning, and new sign-in for a moved source", async () => {
    orgs([acme]);
    backend.on("org_preview", () => keyChange);
    backend.on("org_update", () => ({ ...acme, key_fingerprint: keyChange.key_fingerprint }));
    const { user } = renderApp("/settings");
    const dialog = await preview(user, "Acme Engineering");
    expect(
      within(dialog).getByRole("heading", { name: "Update Acme Engineering" }),
    ).toBeVisible();
    const changes = within(dialog).getByRole("list", { name: "What changes" });
    const [path, key] = within(changes).getAllByRole("listitem");
    expect(path).toHaveTextContent(keyChange.changes[0]);
    expect(path).not.toHaveClass("text-needs");
    expect(key).toHaveTextContent(keyChange.changes[1]);
    expect(key).toHaveClass("text-needs");
    expect(keyChange.key_changed).toBe(true);
    expect(keyChange.source_changed).toBe(true);
    await user.click(within(dialog).getByRole("button", { name: "Continue" }));

    // The path changed: the saved details are not sent to it, so they are asked as at a Join.
    expect(dialog).toHaveTextContent(
      "The source changed, so enter your sign-in details again: Dots does not send the saved " +
        "ones to it.",
    );
    expect(dialog).not.toHaveTextContent("Leave these empty");
    const update = within(dialog).getByRole("button", { name: "Update" });
    expect(update).toBeDisabled();
    await user.type(within(dialog).getByLabelText("Token"), TOKEN);
    noSecretShown();
    await user.click(update);
    expect(backend.callsTo("org_update")).toEqual([
      { orgId: "acme-eng", code: CODE, creds: { kind: "token", token: TOKEN } },
    ]);
    expect(await screen.findByText("Updated Acme Engineering")).toBeVisible();
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    noSecretShown();
  });

  it("keeps the saved sign-in details when the code leaves the source as it is", async () => {
    orgs([acme]);
    const keyOnly = {
      ...keyChange,
      changes: keyChange.changes.slice(-1),
      source_changed: false,
    } satisfies JoinPreview;
    backend.on("org_preview", () => keyOnly);
    backend.on("org_update", () => acme);
    const { user } = renderApp("/settings");
    const dialog = await preview(user, "Acme Engineering");
    await user.click(within(dialog).getByRole("button", { name: "Continue" }));

    expect(dialog).toHaveTextContent("Leave these empty to keep the saved sign-in details.");
    expect(dialog).not.toHaveTextContent("The source changed");
    await user.click(within(dialog).getByRole("button", { name: "Update" }));
    expect(backend.callsTo("org_update")).toEqual([
      { orgId: "acme-eng", code: CODE, creds: null },
    ]);
    expect(await screen.findByText("Updated Acme Engineering")).toBeVisible();
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("warns of a new key by the preview's flag, not by the words of a line", async () => {
    orgs([acme]);
    backend.on("org_preview", () => ({ ...keyChange, key_changed: false }));
    const { user } = renderApp("/settings");
    const dialog = await preview(user, "Acme Engineering");
    const changes = within(dialog).getByRole("list", { name: "What changes" });
    for (const line of within(changes).getAllByRole("listitem")) {
      expect(line).not.toHaveClass("text-needs");
    }
  });

  it("goes back to the preview, with nothing changed, when the new key is declined", async () => {
    orgs([acme]);
    backend.on("org_preview", () => keyChange);
    backend.on("org_update", () => {
      throw { kind: "cancelled", message: "the org was not updated" };
    });
    const { user } = renderApp("/settings");
    const dialog = await preview(user, "Acme Engineering");
    await user.click(within(dialog).getByRole("button", { name: "Continue" }));
    await user.type(within(dialog).getByLabelText("Token"), TOKEN);
    await user.click(within(dialog).getByRole("button", { name: "Update" }));

    expect(
      await within(dialog).findByText("Not updated: you declined the new policy key."),
    ).toBeVisible();
    expect(within(dialog).getByRole("list", { name: "What changes" })).toBeVisible();
    expect(within(dialog).getByRole("button", { name: "Continue" })).toBeVisible();
    expect(backend.callsTo("org_update")).toEqual([
      { orgId: "acme-eng", code: CODE, creds: { kind: "token", token: TOKEN } },
    ]);
    // The typed token went with the answer.
    await user.click(within(dialog).getByRole("button", { name: "Continue" }));
    expect(within(dialog).getByLabelText("Token")).toHaveValue("");
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(within(await row("Acme Engineering")).getByText(acme.key_fingerprint)).toBeVisible();
    noSecretShown();
  });

  it("reads a .dotsorg file and previews its text", async () => {
    orgs([]);
    backend.on("org_preview", () => newCode);
    const { user } = renderApp("/settings");
    await user.click(within(await section()).getByRole("button", { name: "Join an organisation" }));
    const dialog = await screen.findByRole("dialog");
    const text = `${CODE}\n`;
    await user.upload(
      within(dialog).getByLabelText("Or open a .dotsorg file"),
      new File([text], "acme.dotsorg", { type: "application/octet-stream" }),
    );
    expect(await within(dialog).findByText(newCode.key_fingerprint)).toBeVisible();
    expect(backend.callsTo("org_preview")).toEqual([{ code: text }]);
  });

  it("shows the core's error as it is, and clears the secret it was sent with", async () => {
    orgs([]);
    backend.on("org_preview", (args) => {
      if (args.code !== CODE) throw { kind: "invalid", message: "this is not a join code: bad" };
      return newCode;
    });
    const failure = "could not join Acme Engineering: permission denied (403)";
    backend.on("org_join", () => {
      throw { kind: "internal", message: failure };
    });
    const { user } = renderApp("/settings");
    await user.click(within(await section()).getByRole("button", { name: "Join an organisation" }));
    let dialog = await screen.findByRole("dialog");
    await user.type(within(dialog).getByLabelText("Join code"), "nonsense");
    await user.click(within(dialog).getByRole("button", { name: "Preview" }));
    expect(await within(dialog).findByText("this is not a join code: bad")).toBeVisible();
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());

    dialog = await preview(user);
    await user.click(within(dialog).getByRole("button", { name: "Continue" }));
    await user.type(within(dialog).getByLabelText("Token"), TOKEN);
    await user.click(within(dialog).getByRole("button", { name: "Join" }));
    expect(await within(dialog).findByText(failure)).toBeVisible();
    expect(within(dialog).getByLabelText("Token")).toHaveValue("");
    noSecretShown();
  });

  it("refuses a code for an org joined already, or for another org on Update", async () => {
    orgs([acme]);
    backend.on("org_preview", () => ({ ...newCode, existing: true }));
    const { user } = renderApp("/settings");
    let dialog = await preview(user);
    expect(
      await within(dialog).findByText(
        "Acme Engineering is joined already: use Update on it in the list.",
      ),
    ).toBeVisible();
    expect(within(dialog).getByRole("button", { name: "Continue" })).toBeDisabled();
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());

    backend.on("org_preview", () => ({ ...keyChange, org_id: "beta", name: "Beta" }));
    dialog = await preview(user, "Acme Engineering");
    expect(
      await within(dialog).findByText("This join code is for org 'beta', not 'acme-eng'."),
    ).toBeVisible();
    expect(within(dialog).getByRole("button", { name: "Continue" })).toBeDisabled();
  });

  it("asks for the list again when an org is updated", async () => {
    const list = orgs([acme]);
    const { listening } = renderApp("/settings");
    await row("Acme Engineering");
    await listening();
    const revoked = sample.orgs.find((o) => o.status === "revoked")!;
    list.set([revoked]);
    act(() => backend.emit("runtime", { type: "org_updated", org_id: "acme-eng" }));
    expect(await screen.findByText(revoked.status_text)).toBeVisible();
  });
});
