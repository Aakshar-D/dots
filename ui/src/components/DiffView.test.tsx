import { render, screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { WorktreeDiff } from "@/api/types";
import { DiffView, hunkLines } from "@/components/DiffView";
import { sample } from "@/test/fixtures";

describe("the diff view", () => {
  it("keeps the hunks of a patch, down to a trailing blank context line", () => {
    const patch =
      "diff --git a/n.txt b/n.txt\nindex 1..2 100644\n--- a/n.txt\n+++ b/n.txt\n@@ -1,3 +1,3 @@\n-one\n+uno\n two\n \n";
    expect(hunkLines(patch)).toEqual(["@@ -1,3 +1,3 @@", "-one", "+uno", " two", " "]);
    expect(hunkLines("")).toEqual([]);
    expect(hunkLines("diff --git a/x b/x\nnew file mode 100644\n")).toEqual([]);
  });

  it("shows each file with its status and changes, and says when a file is binary", () => {
    render(<DiffView diff={sample.diff} />);
    const [text, binary] = screen.getAllByRole("group");
    expect(within(text).getByText("src/lib.rs")).toBeVisible();
    expect(within(text).getByText("modified")).toBeVisible();
    expect(within(text).getByText("-a")).toBeVisible();
    expect(within(text).getByText("+b")).toBeVisible();
    expect(within(text).queryByText(/^diff --git/)).toBeNull();
    expect(within(binary).getByText("logo.png")).toBeVisible();
    expect(within(binary).getByText("Binary file.")).toBeVisible();
  });

  it("says so when the diff was cut, and when a listed file has nothing to show", () => {
    const cut: WorktreeDiff = {
      truncated: true,
      files: [{ path: "big.txt", status: "added", patch: "", binary: false }],
    };
    render(<DiffView diff={cut} />);
    expect(screen.getByRole("status")).toHaveTextContent("larger than 1 MiB");
    expect(screen.getByText("No text changes to show.")).toBeVisible();
  });

  it("says when nothing changed", () => {
    render(<DiffView diff={{ files: [], truncated: false }} />);
    expect(screen.getByText("No changes.")).toBeVisible();
  });
});
