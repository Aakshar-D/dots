import { describe, expect, it } from "vitest";

import type { Watcher } from "@/api/types";
import {
  defaultWatcherName,
  folderInside,
  watcherEventLine,
  watcherHoursLine,
  watcherState,
  watcherSummary,
  watcherTitle,
} from "@/lib/watchers";
import { localTime, localWeekdayTime } from "@/lib/format";
import { sample } from "@/test/fixtures";

const [folder, email, github] = sample.watchers as [Watcher, Watcher, Watcher];
const configOf = <K extends Watcher["config"]["kind"]>(w: Watcher, _kind: K) =>
  w.config as Extract<Watcher["config"], { kind: K }>;

describe("what a watcher says about itself", () => {
  it("sums each kind up in a line", () => {
    expect(watcherSummary(folder)).toBe("*.pdf in C:\\Users\\u\\Downloads\\Invoices, with subfolders");
    expect(watcherSummary(email)).toBe("billing@acme.com · subject 'invoice' · Gmail label Dots");
    expect(watcherSummary(github)).toBe("owner/repo · issues, PRs");
    const f = configOf(folder, "folder");
    expect(watcherSummary({ ...folder, config: { ...f, pattern: "", recursive: false } })).toBe(
      "Every file in C:\\Users\\u\\Downloads\\Invoices",
    );
    expect(
      watcherSummary({
        ...folder,
        config: { ...f, path: "C:\\Users\\someone\\OneDrive - Company\\Documents\\Accounts\\Invoices" },
      }),
    ).toBe("*.pdf in C:\\…\\Invoices, with subfolders");
    const e = configOf(email, "email");
    expect(
      watcherSummary({ ...email, config: { ...e, from: null, subject: null, label: "INBOX", has_attachment: true } }),
    ).toBe("me@gmail.com · with attachments · Gmail label INBOX");
    const g = configOf(github, "github");
    expect(
      watcherSummary({
        ...github,
        config: { ...g, repos: ["owner/repo", "owner/other"], events: ["issue", "pull_request", "comment", "push", "ci"] },
      }),
    ).toBe("owner/repo, owner/other · issues, PRs, comments, pushes, CI runs");
  });

  it("says whether it is on, paused by the person, or paused by Dots and why", () => {
    expect(watcherState(folder)).toBe("on");
    expect(watcherState({ ...folder, enabled: false })).toBe("paused by you");
    expect(watcherState(email)).toBe(
      "paused: Gmail refused the app password for me@gmail.com, or IMAP is off for it.",
    );
    expect(watcherState(github)).toBe("paused: too many runs");
    expect(watcherState({ ...github, enabled: false })).toBe("paused: too many runs");
  });

  it("names a watcher run's event in a line", () => {
    const file = (event: string) => ({ source: "folder", event, relative: "invoice-12.pdf" });
    expect(watcherEventLine(file("created"))).toBe("New file: invoice-12.pdf");
    expect(watcherEventLine(file("changed"))).toBe("Changed file: invoice-12.pdf");
    expect(watcherEventLine(file("deleted"))).toBe("Deleted file: invoice-12.pdf");
    expect(watcherEventLine({ source: "email", from: "billing@acme.com", subject: "Invoice 12" })).toBe(
      "Email from billing@acme.com: Invoice 12",
    );
    const hub = (event: string, extra = {}) => ({ source: "github", event, number: 42, title: "Crash on start", ...extra });
    expect(watcherEventLine(hub("issue"))).toBe("New issue #42: Crash on start");
    expect(watcherEventLine(hub("pull_request"))).toBe("New PR #42: Crash on start");
    expect(watcherEventLine(hub("comment"))).toBe("New comment on #42: Crash on start");
    expect(watcherEventLine(hub("push", { branch: "main", title: "3 commits" }))).toBe("Push to main: 3 commits");
    expect(watcherEventLine(hub("ci", { branch: "main", title: "CI", conclusion: "failure" }))).toBe(
      "CI failure on main: CI",
    );
    expect(watcherEventLine(null)).toBeNull();
    expect(watcherEventLine({ source: "elsewhere" })).toBeNull();
    const run = { ...sample.run, trigger: "watcher" as const, payload: { ...file("created"), watcher_name: "Invoices folder" } };
    expect(watcherTitle(run)).toBe("Invoices folder · New file: invoice-12.pdf");
    expect(watcherTitle(sample.run)).toBeNull();
  });

  it("names a new watcher after its source and knows a folder inside the dot's", () => {
    expect(defaultWatcherName(folder.config)).toBe("Invoices folder");
    expect(defaultWatcherName(email.config)).toBe("me@gmail.com mail");
    expect(defaultWatcherName(github.config)).toBe("owner/repo");
    expect(folderInside("C:\\work\\in", "C:\\work")).toBe(true);
    expect(folderInside("c:/WORK/in", "C:\\work\\")).toBe(true);
    expect(folderInside("C:\\work", "C:\\work")).toBe(true);
    expect(folderInside("C:\\workshop", "C:\\work")).toBe(false);
    expect(folderInside("D:\\in", "C:\\work")).toBe(false);
  });
});

describe("what a watcher's hours hold back", () => {
  const at = (iso: string) => ({ ...folder, held_count: 4, next_opening: iso });
  // Built with the local constructor, so "today" is the same local day in every time zone.
  const nowToday = new Date(2026, 9, 10, 8, 0);
  const TODAY = new Date(2026, 9, 10, 9, 0).toISOString();
  // 09:00 on Mon 2026-10-12, judged from the middle of Sat 2026-10-10.
  const MONDAY = new Date(2026, 9, 12, 9, 0).toISOString();
  const sat = new Date(2026, 9, 10, 12, 0);

  it("says what is held and until when, today as a time and later as a weekday and time", () => {
    expect(watcherHoursLine(at(TODAY), nowToday)).toBe(`Outside hours · 4 held until ${localTime(TODAY)}`);
    expect(watcherHoursLine({ ...at(TODAY), held_count: 0 }, nowToday)).toBe(
      `Outside hours · until ${localTime(TODAY)}`,
    );
    const later = watcherHoursLine(at(MONDAY), sat);
    expect(later).toBe(`Outside hours · 4 held until ${localWeekdayTime(MONDAY, sat.getTime())}`);
    // Another day names its weekday: not the bare time of today's form.
    expect(later).not.toBe(`Outside hours · 4 held until ${localTime(MONDAY)}`);
  });

  it("is empty without a next opening", () => {
    expect(watcherHoursLine({ ...folder, next_opening: null }, sat)).toBe("");
  });

  it("is empty once the opening it was given has passed", () => {
    expect(watcherHoursLine(at(TODAY), sat)).toBe("");
    expect(watcherHoursLine(at(TODAY), new Date(TODAY))).toBe("");
  });
});
