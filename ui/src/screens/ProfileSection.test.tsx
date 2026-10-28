import { screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Identity } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const { identity } = sample;
const section = () => screen.findByRole("group", { name: "Profile" });

describe("Settings, Profile", () => {
  it("is headed Profile and shows the fingerprint", async () => {
    backend.on("identity_get", () => identity);
    renderApp("/settings");
    const profile = await section();
    expect(within(profile).getByText("Profile", { selector: "legend" })).toBeVisible();
    expect(await within(profile).findByText(identity.fingerprint)).toBeVisible();
  });

  it("copies the public key with Copy my Dots ID", async () => {
    backend.on("identity_get", () => identity);
    const { user } = renderApp("/settings");
    const profile = await section();
    await user.click(await within(profile).findByRole("button", { name: "Copy my Dots ID" }));
    expect(await navigator.clipboard.readText()).toBe(identity.public_key);
  });

  it("saves the name and email", async () => {
    let current: Identity = { ...identity, name: "", email: "" };
    backend.on("identity_get", () => current);
    backend.on("profile_set", (args) => {
      current = { ...current, name: String(args.name), email: String(args.email) };
      return current;
    });
    const { user } = renderApp("/settings");
    const profile = await section();
    await user.type(await within(profile).findByLabelText("Name"), "Asha Rao");
    await user.type(within(profile).getByLabelText("Email"), "asha@acme.com");
    await user.click(within(profile).getByRole("button", { name: "Save profile" }));
    await within(profile).findByDisplayValue("Asha Rao");
    expect(backend.callsTo("profile_set")).toEqual([{ name: "Asha Rao", email: "asha@acme.com" }]);
  });

  it("shows the core's message under the form", async () => {
    backend.on("identity_get", () => identity);
    backend.on("profile_set", () => {
      throw { kind: "invalid", message: "that email has nothing after the @" };
    });
    const { user } = renderApp("/settings");
    const profile = await section();
    await user.type(await within(profile).findByLabelText("Email"), "asha@");
    await user.click(within(profile).getByRole("button", { name: "Save profile" }));
    expect(await within(profile).findByRole("alert")).toHaveTextContent(
      "that email has nothing after the @",
    );
  });
});
