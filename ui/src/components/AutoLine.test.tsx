import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { AutoLine } from "@/components/AutoLine";
import { sample } from "@/test/fixtures";

describe("the Auto line under a reply", () => {
  it("shows the model Auto picked and why", () => {
    const run = { ...sample.run, model: "haiku", model_reason: "light task: check" };
    render(<AutoLine run={{ ...run, model_fallback_from: null, model_tier: "small" }} />);
    expect(screen.getByText("Auto · Haiku — light task: check")).toBeVisible();
  });

  it("marks a fallback and says why on hover", () => {
    render(<AutoLine run={sample.run} />);
    const line = screen.getByText(/^Auto · Sonnet — code work: refactor/);
    expect(line).toHaveTextContent("· fallback");
    expect(line).toHaveAttribute(
      "title",
      "Opus could not be used, so this reply used Sonnet.",
    );
  });

  it("shows nothing for a dot with its own model", () => {
    const run = { ...sample.run, model: "opus", model_tier: null, model_reason: null };
    const { container } = render(<AutoLine run={run} />);
    expect(container).toBeEmptyDOMElement();
  });
});
