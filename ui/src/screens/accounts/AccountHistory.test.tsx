import { screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { AuditView } from "@/api/types";
import { localTime } from "@/lib/format";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const [acme] = sample.orgs;
const [failing, unsent] = sample.audit;

/// Answers `org_list`, `member_list` and `org_audit` like the core for Acme.
function account(view: AuditView) {
  backend.on("org_list", () => [acme]);
  backend.on("member_list", () => sample.members);
  backend.on("workspace_list", () => []);
  backend.on("api_key_list", () => []);
  backend.on("org_audit", () => view);
}

async function row() {
  const view = renderApp("/settings/account");
  const section = await screen.findByRole("group", { name: "Accounts" });
  const item = await within(section).findByRole("listitem", { name: "Acme Engineering" });
  return { ...view, item };
}

async function historyTab() {
  const view = await row();
  await view.user.click(within(view.item).getByRole("tab", { name: "History" }));
  const panel = await within(view.item).findByRole("tabpanel");
  return { ...view, panel };
}

describe("Settings, Accounts, History", () => {
  it("lists the account's records newest first, in words, with whether each was sent", async () => {
    account({ ...failing, send: { ...failing.send, last_error: null } });
    const { panel } = await historyTab();
    const table = await within(panel).findByRole("table", { name: "History of Acme Engineering" });
    const rows = within(table).getAllByRole("row").slice(1);
    const cells = rows.map((r) =>
      within(r)
        .getAllByRole("cell")
        .slice(1)
        .map((c) => c.textContent),
    );
    const refused = "the policy does not allow it";
    const dropped = "the queue was full; the oldest records were dropped";
    expect(cells).toEqual([
      ["Refused", "PROD_TOKEN", "acme-release", "github", refused, "Waiting to send"],
      ["Used", "GITHUB_TOKEN", "acme-release", "github", "", "Sent"],
      ["Expired", "", "", "", dropped, "Dropped unsent"],
    ]);
    expect(panel).toHaveTextContent("Records go to https://audit.acme.example/dots.");
    expect(panel).not.toHaveTextContent("last send failed");
    expect(backend.callsTo("org_audit")).toContainEqual({ orgId: "acme-eng" });
  });

  it("says when sending fails, on History and on Overview", async () => {
    account(failing);
    const { item, user } = await row();
    const line =
      `Audit: 2 records waiting; last send failed ${localTime(failing.send.failed_at)}: ` +
      "the endpoint answered 503";
    expect(await within(item).findByText(line)).toBeInTheDocument();
    await user.click(within(item).getByRole("tab", { name: "History" }));
    const panel = await within(item).findByRole("tabpanel");
    expect(await within(panel).findByText(line)).toBeInTheDocument();
  });

  it("an account with no audit endpoint keeps its records on this PC", async () => {
    account(unsent);
    const { panel } = await historyTab();
    expect(
      await within(panel).findByText(/has no audit endpoint: these records stay on this PC/),
    ).toBeInTheDocument();
    const table = within(panel).getByRole("table", { name: "History of Acme Engineering" });
    expect(table).toHaveTextContent("Offline");
    expect(table).toHaveTextContent("Kept on this PC");
  });

  it("an account with nothing recorded says so", async () => {
    account({ ...unsent, history: [] });
    const { panel } = await historyTab();
    expect(await within(panel).findByText("Nothing recorded yet.")).toBeInTheDocument();
  });
});
