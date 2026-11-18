import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it } from "vitest";

import type { ModelOption } from "@/api/types";
import { CUSTOM, ModelPicker, withoutAuto } from "@/components/ModelPicker";
import { claudeModels } from "@/test/fixtures";

const option = (id: string): ModelOption => ({ id, label: id.toUpperCase(), hint: "" });

/// A picker that holds its own value, over the list `options`.
function Held(props: { options: ModelOption[]; start: string }) {
  const [value, setValue] = useState(props.start);
  return (
    <>
      <ModelPicker id="m" ariaLabel="Model" value={value} onChange={setValue} options={props.options} />
      <output aria-label="Saved">{value}</output>
    </>
  );
}

describe("the model picker", () => {
  it("shows a picked model as Custom once the list no longer has it", async () => {
    const user = userEvent.setup();
    const view = render(<Held options={[option("a"), option("b")]} start="a" />);
    await user.selectOptions(screen.getByLabelText("Model"), "B");
    expect(screen.getByLabelText("Model")).toHaveValue("b");
    // Another endpoint, whose list lacks it.
    view.rerender(<Held options={[option("a"), option("c")]} start="a" />);
    expect(screen.getByLabelText("Model")).toHaveValue(CUSTOM);
    expect(screen.getByRole("textbox", { name: "Custom model ID" })).toHaveValue("b");
    expect(screen.getByLabelText("Saved")).toHaveTextContent("b");
  });

  it("keeps a custom model id without the spaces around it", async () => {
    const user = userEvent.setup();
    render(<Held options={[option("a")]} start="a" />);
    await user.selectOptions(screen.getByLabelText("Model"), "Custom…");
    const box = screen.getByRole("textbox", { name: "Custom model ID" });
    await user.clear(box);
    await user.type(box, "  x-1 ");
    expect(screen.getByLabelText("Saved")).toHaveTextContent(/^x-1$/);
  });

  it("offers Auto first for a Claude sol, with what it does", async () => {
    const user = userEvent.setup();
    render(<Held options={claudeModels} start="sonnet" />);
    const select = screen.getByLabelText("Model");
    const names = [...select.querySelectorAll("option")].map((o) => o.textContent);
    expect(names).toEqual(["Auto", "Opus", "Sonnet", "Haiku", "Custom…"]);
    await user.selectOptions(select, "Auto");
    expect(screen.getByLabelText("Saved")).toHaveTextContent(/^auto$/);
    expect(screen.getByText(/Picks Haiku, Sonnet or Opus for each task/)).toBeVisible();
  });

  it("leaves Auto out of a list of fixed models", () => {
    expect(withoutAuto(claudeModels).map((m) => m.id)).toEqual(["opus", "sonnet", "haiku"]);
  });
});
