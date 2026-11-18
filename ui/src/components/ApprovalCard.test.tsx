import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Approval } from "@/api/types";
import { ApprovalCard } from "@/components/ApprovalCard";
import { createQueryClient } from "@/lib/query";
import { sample } from "@/test/fixtures";

const [pending] = sample.inbox.approvals;

function show(approval: Approval) {
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <ApprovalCard approval={approval} dotName="Researcher" />
    </QueryClientProvider>,
  );
}

/// What the card says the call asks for `key`.
const asked = (asks: HTMLElement, key: string) =>
  within(asks).getByText(key, { selector: "dt" }).nextElementSibling as HTMLElement;

const asking = (tool: string, input: unknown): Approval => ({ ...pending!, tool, input });

describe("an approval of a maker's call", () => {
  it("names the sol to be made, and lists what is asked as plain text", async () => {
    show(
      asking("MakeDots", {
        tool: "make_dot",
        input: {
          name: "Blog Watcher",
          purpose: "Reads the feeds",
          instructions: "Read the feeds.\n[x](javascript:alert(1)) <img src=x onerror=alert(2)>",
          browser: true,
        },
      }),
    );
    const card = await screen.findByRole("article", { name: "Approval for MakeDots" });
    expect(within(card).getByLabelText("What this does")).toHaveTextContent(
      "Researcher wants to make a sol: Blog Watcher",
    );
    const asks = within(card).getByLabelText("What it asks");
    expect(asked(asks, "name")).toHaveTextContent("Blog Watcher");
    expect(asked(asks, "purpose")).toHaveTextContent("Reads the feeds");
    expect(asked(asks, "browser")).toHaveTextContent("true");
    // A sol wrote the instructions: text only.
    expect(asks).toHaveTextContent("[x](javascript:alert(1)) <img src=x onerror=alert(2)>");
    expect(card.querySelector("img, a")).toBeNull();
    // No raw JSON of the call.
    expect(card.querySelector("pre")).toBeNull();
    expect(within(card).getByRole("button", { name: "Approve" })).toBeVisible();
  });

  it("names the made sol a change, a delete or a job is for, as the core found it", async () => {
    show(
      asking("MakeDots", {
        tool: "give_job",
        input: { dot: "blog watcher", message: "Read today's feeds." },
        dot: { id: "01J0MADE", name: "Blog Watcher" },
      }),
    );
    expect(await screen.findByLabelText("What this does")).toHaveTextContent(
      "Researcher wants to give a job to the sol Blog Watcher",
    );
    expect(asked(screen.getByLabelText("What it asks"), "message")).toHaveTextContent(
      "Read today's feeds.",
    );
  });

  it("shows a shape it does not know as every other call", async () => {
    show(asking("MakeDots", { tool: "make_dot", input: "not an object" }));
    const card = await screen.findByRole("article", { name: "Approval for MakeDots" });
    expect(within(card).queryByLabelText("What this does")).toBeNull();
    expect(card.querySelector("pre")).toHaveTextContent('"input": "not an object"');
  });

  it("leaves every other tool's card as it was", async () => {
    show(asking("Bash", { command: "ls" }));
    const card = await screen.findByRole("article", { name: "Approval for Bash" });
    expect(within(card).queryByLabelText("What it asks")).toBeNull();
    expect(card.querySelector("pre")).toHaveTextContent('"command": "ls"');
  });
});
