import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { MembersView, OrgSummary } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const [acme] = sample.orgs; // Acme Engineering, active, this PC an Admin
const [bensRequest, troubled] = sample.requestPreviews;
const OWNER = "Owners can only be changed with the account owner's key, using dots-admin.";
const NOT_A_SIGNER =
  "Acme Engineering: your Dots ID isn't one of the account's signing admins yet. Ask the " +
  "owner to add it with dots-admin policy admins add.";
const REQUEST = "dotsreq_c2FtcGxlLXJlcXVlc3QtY29kZQ";
// The list as it stands, with no draft unless a test gives one.
const list: MembersView = { ...sample.members, draft: null };

/// Answers `org_list` with `org` and `member_list` with what the test holds, like the core.
function account(view: MembersView, org: OrgSummary = acme) {
  let current = view;
  backend.on("org_list", () => [org]);
  backend.on("member_list", () => current);
  return {
    set: (next: MembersView) => {
      current = next;
    },
  };
}

/// Opens Settings, then the Members tab of Acme Engineering.
async function membersTab() {
  const view = renderApp("/settings");
  const section = await screen.findByRole("group", { name: "Accounts" });
  const row = await within(section).findByRole("listitem", {
    name: "Acme Engineering",
  });
  await view.user.click(within(row).getByRole("tab", { name: "Members" }));
  const panel = await within(row).findByRole("tabpanel");
  await within(panel).findByRole("list", { name: "Members" });
  return { ...view, panel };
}

const member = (panel: HTMLElement, name: string) =>
  within(within(panel).getByRole("list", { name: "Members" })).getByRole("listitem", { name });

describe("Settings, Accounts, Members", () => {
  it("viewer sees members but no controls", async () => {
    account(
      { ...list, can_manage: false, can_sign: false, sign_blocker: "refused" },
      { ...acme, role: "viewer" },
    );
    const { panel } = await membersTab();
    const why = "Acme Engineering: a Viewer can't manage members. Ask an Admin or Owner.";
    const asha = member(panel, "Asha Rao");
    expect(asha).toHaveTextContent("asha@acme.com");
    expect(asha).toHaveTextContent("Admin");
    expect(asha).toHaveTextContent("platform: Admin");
    expect(asha).toHaveTextContent("AB12 CD34 EF56 7890");
    expect(asha).toHaveTextContent("This PC");
    for (const name of ["Invite", "Approve request"]) {
      const button = within(panel).getByRole("button", { name });
      expect(button).toBeDisabled();
      expect(button).toHaveAttribute("title", why);
    }
    expect(within(asha).getByLabelText("Role of Asha Rao")).toBeDisabled();
    expect(within(asha).getByLabelText("Role of Asha Rao")).toHaveAttribute("title", why);
    expect(within(asha).getByRole("button", { name: "Remove Asha Rao" })).toBeDisabled();
    for (const pc of within(asha).getAllByRole("button", {
      name: /Remove this PC/,
    })) {
      expect(pc).toBeDisabled();
      expect(pc).toHaveAttribute("title", why);
    }
    expect(within(panel).getByRole("button", { name: "Revoke" })).toBeDisabled();
  });

  it("admin invites and sees the code", async () => {
    account(list);
    backend.on("invite_create", () => sample.createdInvite);
    const { user, panel } = await membersTab();
    // Only the open invite shows, with Revoke.
    const invites = within(panel).getByRole("list", { name: "Open invites" });
    expect(invites).toHaveTextContent("ben@acme.com");
    expect(invites).not.toHaveTextContent("cy@acme.com");
    const before = backend.callsTo("member_list").length;

    await user.click(within(panel).getByRole("button", { name: "Invite" }));
    const dialog = await screen.findByRole("dialog");
    await user.type(within(dialog).getByLabelText("Email"), "ben@acme.com");
    await user.selectOptions(within(dialog).getByLabelText("Role"), "admin");
    await user.click(within(dialog).getByRole("button", { name: "Create invite" }));
    expect(await within(dialog).findByLabelText("Invite code")).toHaveValue(
      sample.createdInvite.code,
    );
    expect(backend.callsTo("invite_create")).toEqual([
      { orgId: "acme-eng", email: "ben@acme.com", role: "admin" },
    ]);
    await user.click(within(dialog).getByRole("button", { name: "Copy" }));
    expect(await navigator.clipboard.readText()).toBe(sample.createdInvite.code);
    expect(within(dialog).getByRole("link", { name: "Save as .dotsorg" })).toHaveAttribute(
      "download",
      "acme-eng.dotsorg",
    );
    // The list (with its invites) is asked for again.
    await waitFor(() => expect(backend.callsTo("member_list").length).toBeGreaterThan(before));
  });

  it("revokes an open invite", async () => {
    account(list);
    backend.on("invite_revoke", () => null);
    const { user, panel } = await membersTab();
    await user.click(within(panel).getByRole("button", { name: "Revoke" }));
    expect(backend.callsTo("invite_revoke")).toEqual([{ orgId: "acme-eng", inviteId: "inv-1" }]);
  });

  it("approve shows the request and publishes", async () => {
    account(list);
    backend.on("request_inspect", () => bensRequest);
    backend.on("member_edit", () => ({ outcome: "published", summary: acme }));
    const { user, panel } = await membersTab();
    await user.click(within(panel).getByRole("button", { name: "Approve request" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByLabelText("Request code"));
    await user.paste(REQUEST);
    await user.click(within(dialog).getByRole("button", { name: "Check" }));
    expect(await within(dialog).findByText("Ben Okoye")).toBeVisible();
    expect(dialog).toHaveTextContent("ben@acme.com");
    expect(dialog).toHaveTextContent("5A5A 6B6B 7C7C 8D8D");
    expect(dialog).toHaveTextContent("Member");
    expect(backend.callsTo("request_inspect")).toEqual([{ orgId: "acme-eng", code: REQUEST }]);
    // He has an invite: no confirmation is asked.
    expect(within(dialog).queryByLabelText("This person has no invite from this PC")).toBeNull();
    await user.click(within(dialog).getByRole("button", { name: "Approve" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(backend.callsTo("member_edit")).toEqual([
      {
        orgId: "acme-eng",
        baseSeq: 3,
        edit: {
          kind: "approve",
          request: REQUEST,
          role: null,
          confirm_no_invite: false,
        },
      },
    ]);
  });

  it("approve without an invite asks to confirm and for a role", async () => {
    account(list);
    backend.on("request_inspect", () => ({
      ...bensRequest,
      invite: null,
      role: null,
      age_recipient: "age1samplerecipient",
    }));
    backend.on("member_edit", () => ({ outcome: "published", summary: acme }));
    const { user, panel } = await membersTab();
    await user.click(within(panel).getByRole("button", { name: "Approve request" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByLabelText("Request code"));
    await user.paste(REQUEST);
    await user.click(within(dialog).getByRole("button", { name: "Check" }));
    // The age key the SOPS file must be encrypted to.
    expect(await within(dialog).findByText("age1samplerecipient")).toBeVisible();
    const approve = within(dialog).getByRole("button", { name: "Approve" });
    expect(approve).toBeDisabled();
    await user.click(within(dialog).getByLabelText("This person has no invite from this PC"));
    await user.selectOptions(within(dialog).getByLabelText("Role"), "viewer");
    await user.click(approve);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(backend.callsTo("member_edit")[0].edit).toEqual({
      kind: "approve",
      request: REQUEST,
      role: "viewer",
      confirm_no_invite: true,
    });
  });

  it("approve problem is shown and Approve disabled", async () => {
    account(list);
    backend.on("request_inspect", () => troubled);
    const { user, panel } = await membersTab();
    await user.click(within(panel).getByRole("button", { name: "Approve request" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByLabelText("Request code"));
    await user.paste(REQUEST);
    await user.click(within(dialog).getByRole("button", { name: "Check" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "this request is older than the list",
    );
    expect(within(dialog).getByRole("button", { name: "Approve" })).toBeDisabled();
  });

  it("owner rows are locked with the owner text", async () => {
    account(list);
    const { panel } = await membersTab();
    const olu = member(panel, "Olu Owner");
    const role = within(olu).getByLabelText("Role of Olu Owner");
    expect(role).toBeDisabled();
    expect(role).toHaveAttribute("title", OWNER);
    const remove = within(olu).getByRole("button", {
      name: "Remove Olu Owner",
    });
    expect(remove).toBeDisabled();
    expect(remove).toHaveAttribute("title", OWNER);
    const pc = within(olu).getByRole("button", { name: /Remove this PC/ });
    expect(pc).toBeDisabled();
    expect(pc).toHaveAttribute("title", OWNER);
    // Asha, an Admin, is not locked.
    expect(within(member(panel, "Asha Rao")).getByLabelText("Role of Asha Rao")).toBeEnabled();
  });

  it("not a signing admin explains why", async () => {
    const asha = list.members[1];
    account({
      ...list,
      can_sign: false,
      sign_blocker: NOT_A_SIGNER,
      members: [list.members[0], { ...asha, keys: asha.keys.map((k) => ({ ...k, signs: false })) }],
    });
    const { panel } = await membersTab();
    expect(member(panel, "Asha Rao")).toHaveTextContent(
      "Can sign the list once the owner adds their Dots ID to the policy's admins.",
    );
    const invite = within(panel).getByRole("button", { name: "Invite" });
    expect(invite).toBeDisabled();
    expect(invite).toHaveAttribute("title", NOT_A_SIGNER);
    expect(within(member(panel, "Asha Rao")).getByLabelText("Role of Asha Rao")).toHaveAttribute(
      "title",
      NOT_A_SIGNER,
    );
  });

  it("conflict refetches and shows the text", async () => {
    account(list);
    const changed =
      "Acme Engineering: the member list changed (now #4) while you were editing. Check it and " +
      "make the change again.";
    backend.on("member_edit", () => {
      throw { kind: "conflict", message: changed };
    });
    const { user, panel } = await membersTab();
    const before = backend.callsTo("member_list").length;
    await user.selectOptions(
      within(member(panel, "Asha Rao")).getByLabelText("Role of Asha Rao"),
      "member",
    );
    expect(await within(panel).findByRole("alert")).toHaveTextContent(changed);
    expect(backend.callsTo("member_edit")).toEqual([
      {
        orgId: "acme-eng",
        baseSeq: 3,
        edit: { kind: "set_role", member_id: "m-asha", role: "member" },
      },
    ]);
    await waitFor(() => expect(backend.callsTo("member_list").length).toBeGreaterThan(before));
  });

  it("remove asks first, and says the source must cut them off too", async () => {
    account(list);
    backend.on("member_edit", () => ({ outcome: "published", summary: acme }));
    const { user, panel } = await membersTab();
    await user.click(
      within(member(panel, "Asha Rao")).getByRole("button", {
        name: "Remove Asha Rao",
      }),
    );
    const ask = await screen.findByRole("alertdialog");
    expect(ask).toHaveTextContent(
      "Also revoke their access in the source: the list stops Dots honouring them, only the " +
        "source cuts them off.",
    );
    await user.click(within(ask).getByRole("button", { name: "Remove" }));
    await waitFor(() => expect(backend.callsTo("member_edit")).toHaveLength(1));
    expect(backend.callsTo("member_edit")[0].edit).toEqual({
      kind: "remove",
      member_id: "m-asha",
    });
  });

  it("removes one PC of a member", async () => {
    account(list);
    backend.on("member_edit", () => ({ outcome: "published", summary: acme }));
    const { user, panel } = await membersTab();
    const asha = member(panel, "Asha Rao");
    await user.click(
      within(asha).getByRole("button", {
        name: "Remove this PC 99FF 88EE 77DD 66CC",
      }),
    );
    await waitFor(() => expect(backend.callsTo("member_edit")).toHaveLength(1));
    expect(backend.callsTo("member_edit")[0].edit).toEqual({
      kind: "remove_key",
      member_id: "m-asha",
      key: "c3NhbXBsZS1zZWNvbmQtcGM",
    });
  });

  it("draft banner retries and discards", async () => {
    const draft = sample.members.draft!;
    const state = account({ ...list, draft });
    backend.on("roster_draft_retry", () => ({ outcome: "draft", draft }));
    backend.on("roster_draft_discard", () => {
      state.set(list);
      return null;
    });
    const { user, panel } = await membersTab();
    const banner = within(panel).getByRole("region", {
      name: "Unpublished member list",
    });
    expect(banner).toHaveTextContent("Remove Ben Okoye");
    expect(banner).toHaveTextContent("the source could not be reached (connection refused)");
    // Dots publishes this list itself: nothing to copy by hand.
    expect(within(banner).queryByRole("button", { name: "Copy signed list" })).toBeNull();
    await user.click(within(banner).getByRole("button", { name: "Retry" }));
    expect(backend.callsTo("roster_draft_retry")).toEqual([{ orgId: "acme-eng" }]);
    await user.click(within(banner).getByRole("button", { name: "Discard" }));
    expect(backend.callsTo("roster_draft_discard")).toEqual([{ orgId: "acme-eng" }]);
    await waitFor(() =>
      expect(
        within(panel).queryByRole("region", {
          name: "Unpublished member list",
        }),
      ).toBeNull(),
    );
  });

  it("sops draft offers copy", async () => {
    const draft = sample.members.draft!;
    account({ ...list, publishes: false, draft });
    const { user, panel } = await membersTab();
    const banner = within(panel).getByRole("region", {
      name: "Unpublished member list",
    });
    await user.click(within(banner).getByRole("button", { name: "Copy signed list" }));
    expect(await navigator.clipboard.readText()).toBe(draft.signed);
  });

  it("shows no Members tab for an account this PC is not on", async () => {
    backend.on("org_list", () => [sample.orgs[2]]); // waiting
    renderApp("/settings");
    const section = await screen.findByRole("group", { name: "Accounts" });
    const row = await within(section).findByRole("listitem", {
      name: "Acme Engineering",
    });
    expect(within(row).queryByRole("tab", { name: "Members" })).toBeNull();
  });
});
