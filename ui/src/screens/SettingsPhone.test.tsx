import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { PairingView, PhoneDevice, PhoneStatus, Settings } from "@/api/types";
import { settingsErrors } from "@/screens/SettingsScreen";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const off: Settings = { ...sample.settings, phone_enabled: false };
const on: Settings = { ...sample.settings, phone_enabled: true };

const listening: PhoneStatus = {
  running: true,
  port: 47323,
  addresses: ["https://192.168.1.20:47323", "https://10.0.0.5:47323"],
  fingerprint: "AB:CD:EF:01",
  error: null,
  pairing: false,
};

const pairing: PairingView = {
  url: "https://192.168.1.20:47323/#t=token&fp=ABCDEF01",
  code: "ABCD-EFGH",
  address: "https://192.168.1.20:47323",
  fingerprint: "AB:CD:EF:01",
  expires_at: "2026-11-06T10:05:00.000Z",
  qr_size: 29,
  qr_path: "M4 4h1v1h-1z",
};

const iphone: PhoneDevice = {
  id: "01J0PHN",
  name: "iPhone",
  created_at: "2026-11-06T09:00:00.000Z",
  last_seen_at: "2026-11-06T09:30:00.000Z",
  last_ip: "192.168.1.31",
};

/// A backend whose settings are `stored` and whose phone view answers as given.
function phone(stored: Settings, status: PhoneStatus, devices: PhoneDevice[] = []) {
  let current = stored;
  let shown = devices;
  backend.on("get_settings", () => current);
  backend.on("update_settings", (args) => {
    current = args.settings as Settings;
    return { settings: current, restart_required: false };
  });
  backend.on("phone_status", () => status);
  backend.on("phone_devices", () => shown);
  backend.on("phone_actions", () => [
    {
      device_id: iphone.id,
      device_name: "iPhone",
      action: "approve",
      target: "01J0APR",
      summary: "Approved Bash",
      at: "2026-11-06T09:31:00.000Z",
    },
  ]);
  backend.on("phone_pair_start", () => pairing);
  backend.on("phone_pair_cancel", () => null);
  backend.on("phone_device_remove", (args) => {
    shown = shown.filter((d) => d.id !== args.id);
    return null;
  });
}

describe("Settings > Phone", () => {
  it("is off by default, and Save turns it on", async () => {
    phone(off, { ...listening, running: false, addresses: [], fingerprint: null });
    const { user } = renderApp("/settings/phone");
    const toggle = await screen.findByLabelText("Answer from my phone");
    expect(toggle).not.toBeChecked();
    expect(screen.queryByRole("button", { name: "Pair a phone" })).toBeNull();
    expect(screen.getByText(/Use it only on a network you trust/)).toBeVisible();
    expect(screen.getByText(/Windows may ask whether Sage may use the network/)).toBeVisible();

    await user.click(toggle);
    expect(screen.getByText("Save to turn it on.")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(backend.callsTo("update_settings")).toEqual([
        { settings: { ...off, phone_enabled: true } },
      ]),
    );
  });

  it("pairs a phone with a QR code, a code and the certificate's fingerprint", async () => {
    phone(on, listening);
    const { user } = renderApp("/settings/phone");
    expect(await screen.findByText(/Listening on https:\/\/192\.168\.1\.20:47323/)).toBeVisible();
    // The QR code can name either address.
    await user.selectOptions(
      screen.getByLabelText("Address for the QR code"),
      "https://10.0.0.5:47323",
    );
    await user.click(screen.getByRole("button", { name: "Pair a phone" }));
    expect(backend.callsTo("phone_pair_start")).toEqual([{ address: "https://10.0.0.5:47323" }]);
    const qr = await screen.findByRole("img", { name: "QR code to pair a phone" });
    expect(qr.querySelector("path")).toHaveAttribute("d", pairing.qr_path);
    expect(screen.getByLabelText("Pairing code")).toHaveTextContent("ABCD-EFGH");
    expect(screen.getByLabelText("Certificate fingerprint")).toHaveTextContent("AB:CD:EF:01");

    await user.click(screen.getByRole("button", { name: "Cancel pairing" }));
    expect(backend.callsTo("phone_pair_cancel")).toHaveLength(1);
    expect(await screen.findByRole("button", { name: "Pair a phone" })).toBeVisible();
  });

  it("lists paired phones and removes one after asking", async () => {
    phone(on, listening, [iphone]);
    const { user } = renderApp("/settings/phone");
    const row = await screen.findByRole("listitem", { name: "iPhone" });
    expect(within(row).getByText(/from 192\.168\.1\.31/)).toBeVisible();
    expect(screen.getByText(/Approved Bash/)).toBeVisible();

    await user.click(within(row).getByRole("button", { name: "Remove iPhone" }));
    const dialog = await screen.findByRole("alertdialog");
    await user.click(within(dialog).getByRole("button", { name: "Remove" }));
    await waitFor(() => expect(backend.callsTo("phone_device_remove")).toEqual([{ id: iphone.id }]));
    expect(await screen.findByText("No phones are paired.")).toBeVisible();
  });

  it("says why it is not listening", async () => {
    phone(on, {
      ...listening,
      running: false,
      addresses: [],
      fingerprint: null,
      error: "This PC has no private network address.",
    });
    renderApp("/settings/phone");
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Not listening: This PC has no private network address.",
    );
    expect(screen.queryByRole("button", { name: "Pair a phone" })).toBeNull();
  });

  it("checks the phone port", () => {
    const base = { port: 47321, max_concurrent_runs: 2 };
    expect(settingsErrors({ ...base, phone_port: 47321 }).phonePort).toBe(
      "The phone port must differ from the local server's port.",
    );
    expect(settingsErrors({ ...base, phone_port: 0 }).phonePort).toBeDefined();
    expect(settingsErrors({ ...base, phone_port: 47323 }).phonePort).toBeUndefined();
  });
});
