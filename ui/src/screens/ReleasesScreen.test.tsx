import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { PageState, PublisherStatus } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const [needed, halfDone, building] = sample.pageStates as [PageState, PageState, PageState];
const publisher: PublisherStatus = {
  token: "GITHUB_PUBLISH",
  state: { state: "publisher", login: "shraddesh" },
};

function releases(page: PageState) {
  backend.on("publisher_status", () => publisher);
  backend.on("releases_page", () => page);
  return renderApp("/releases");
}

describe("the Releases page", () => {
  it("shows the rail icon only to a publisher", async () => {
    backend.on("publisher_status", () => publisher);
    const first = renderApp("/home");
    const nav = await screen.findByRole("navigation", { name: "Main" });
    expect(await within(nav).findByRole("link", { name: "Releases" })).toBeVisible();
    first.unmount();
    backend.on("publisher_status", () => ({
      token: "GITHUB_PUBLISH",
      state: { state: "refused", reason: "This token needs the repo and workflow scopes." },
    }));
    renderApp("/home");
    const rail = await screen.findByRole("navigation", { name: "Main" });
    await within(rail).findByRole("link", { name: "Settings" });
    expect(within(rail).queryByRole("link", { name: "Releases" })).toBeNull();
  });

  it("walks through the setup in order and ticks each step", async () => {
    backend.on("release_make_key", () => "UFVCTElD");
    backend.on("release_save_key", () => "C:/Users/me/dots-updates.key");
    backend.on("release_put_secrets", () => null);
    backend.on("release_open_key_pull", () => ({
      number: 31,
      url: "https://github.com/Aakshar-D/dots/pull/31",
    }));
    const { user } = releases(needed);
    expect(await screen.findByText("Releases aren't set up yet.")).toBeVisible();
    const put = screen.getByRole("button", { name: "Put it in GitHub's secrets" });
    expect(put).toBeDisabled();
    await user.type(screen.getByLabelText("Password"), "correct horse battery");
    await user.click(screen.getByRole("button", { name: "Make the signing key" }));
    await waitFor(() =>
      expect(backend.callsTo("release_make_key")).toEqual([{ password: "correct horse battery" }]),
    );
    expect(
      screen.getByText(
        "If this key and its password are both lost, no installed Dots can ever take another update.",
      ),
    ).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Save key file…" }));
    expect(await screen.findByText(/C:\/Users\/me\/dots-updates\.key/)).toBeVisible();
    expect(put).toBeDisabled();
    await user.click(
      screen.getByLabelText(
        "I have saved the key file and its password somewhere other than this PC",
      ),
    );
    await user.click(put);
    await waitFor(() => expect(backend.callsTo("release_put_secrets")).toHaveLength(1));
    await user.click(screen.getByRole("button", { name: "Put the public key into Dots" }));
    expect(await screen.findByRole("link", { name: /pull request #31/ })).toHaveAttribute(
      "href",
      "https://github.com/Aakshar-D/dots/pull/31",
    );
  });

  it("says a half set-up repository is left alone", async () => {
    releases(halfDone);
    expect(
      await screen.findByText(
        "GitHub already has a signing key. Ask whoever set it up for the public key, or remove both secrets on GitHub to start again.",
      ),
    ).toBeVisible();
    expect(screen.queryByRole("button", { name: "Make the signing key" })).toBeNull();
  });

  it("publishes a trial after asking with the exact words", async () => {
    backend.on("release_publish_trial", () => building.trial);
    const { user } = releases({ ...building, trial: null, promotion: null });
    const version = await screen.findByLabelText("Version");
    expect(version).toHaveValue("0.3.0");
    await user.click(screen.getByRole("button", { name: "Publish trial" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog).toHaveTextContent(
      "Publish 0.3.0-rc.1 from master as it is now? It goes only to publishers' Dots.",
    );
    await user.click(within(dialog).getByRole("button", { name: "Publish trial" }));
    await waitFor(() =>
      expect(backend.callsTo("release_publish_trial")).toEqual([{ version: "0.3.0" }]),
    );
  });

  it("refuses a version the core refuses", async () => {
    backend.on("release_publish_trial", () => {
      throw { kind: "invalid", message: "0.2.0 is not above the live 0.2.0." };
    });
    const { user } = releases({ ...building, trial: null, promotion: null });
    const version = await screen.findByLabelText("Version");
    await user.clear(version);
    await user.type(version, "0.2.0");
    await user.click(screen.getByRole("button", { name: "Publish trial" }));
    await user.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Publish trial" }),
    );
    expect(await screen.findByText("0.2.0 is not above the live 0.2.0.")).toBeVisible();
  });

  it("shows the build's stage and links to the run", async () => {
    releases(building);
    expect(await screen.findByText(/Trial v0\.3\.0-rc\.1/)).toBeVisible();
    expect(screen.getByText(/Testing/)).toBeVisible();
    expect(screen.getAllByRole("link", { name: "View on GitHub" })[0]).toHaveAttribute(
      "href",
      "https://github.com/Aakshar-D/dots/actions/runs/7",
    );
  });

  it("says how many changes master has past the trial", async () => {
    const first = releases(building);
    expect(
      await screen.findByText(
        "master has 2 changes newer than this trial; publish a new trial to include them",
      ),
    ).toBeVisible();
    first.unmount();
    releases({ ...building, trial: { ...building.trial!, newer_on_master: 1 } });
    expect(
      await screen.findByText(
        "master has 1 change newer than this trial; publish a new trial to include them",
      ),
    ).toBeVisible();
  });

  it("saves the notes and promotes after asking", async () => {
    backend.on("release_save_notes", () => null);
    backend.on("release_promote", () => null);
    const { user } = releases(building);
    const notes = await screen.findByLabelText("What's new");
    await user.clear(notes);
    await user.type(notes, "- Better");
    await user.click(screen.getByRole("button", { name: "Save notes" }));
    await waitFor(() =>
      expect(backend.callsTo("release_save_notes")).toEqual([
        { version: "0.3.0", notes: "- Better" },
      ]),
    );
    await user.click(screen.getByRole("button", { name: "Promote 0.3.0 to everyone" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog).toHaveTextContent(
      "Every Dots will be offered 0.3.0 within six hours. It is exactly the code of 0.3.0-rc.1.",
    );
    await user.click(within(dialog).getByRole("button", { name: "Promote" }));
    await waitFor(() =>
      expect(backend.callsTo("release_promote")).toEqual([{ version: "0.3.0" }]),
    );
  });

  it("offers Retry after a failed promote, and Drop trial and Withdraw ask first", async () => {
    backend.on("release_retry", () => null);
    backend.on("release_drop_trial", () => null);
    backend.on("release_withdraw", () => null);
    const { user } = releases(building);
    expect(await screen.findByText(/Failed at Build the installer/)).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Retry" }));
    await waitFor(() => expect(backend.callsTo("release_retry")).toEqual([{ tag: "v0.2.1" }]));
    await user.click(screen.getByRole("button", { name: "Drop trial" }));
    await user.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Drop trial" }),
    );
    await waitFor(() =>
      expect(backend.callsTo("release_drop_trial")).toEqual([{ version: "0.3.0" }]),
    );
    await user.click(screen.getByRole("button", { name: "Withdraw 0.2.0…" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog).toHaveTextContent("No more Dots will take 0.2.0.");
    await user.click(within(dialog).getByRole("button", { name: "Withdraw" }));
    await waitFor(() =>
      expect(backend.callsTo("release_withdraw")).toEqual([{ version: "0.2.0" }]),
    );
  });
});
