import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { StatusPill } from "./StatusPill";

describe("StatusPill", () => {
  it.each([
    ["working", "Working"],
    ["needs", "Needs you"],
    ["idle", "Idle"],
    ["up", "Up"],
    ["stopping", "Stopping"],
    ["crashed", "Crashed"],
    ["exited", "Exited"],
    ["notStarted", "Not started"],
  ] as const)("says %s as %s", (state, word) => {
    render(<StatusPill state={state} kind="agent" />);
    expect(screen.getByText(word)).toBeInTheDocument();
  });

  it("says Running for a working terminal", () => {
    render(<StatusPill state="working" kind="terminal" />);
    expect(screen.getByText("Running")).toBeInTheDocument();
  });

  it("shows the time in the state after the word", () => {
    render(<StatusPill state="needs" sinceMs={Date.now() - 64 * 60_000} />);
    expect(screen.getByText("1h 4m")).toBeInTheDocument();
  });

  it("counts sub-agents and shells inside a Working pill", () => {
    render(<StatusPill state="working" subagents={2} shells={1} />);
    expect(screen.getByTitle("2 sub-agents running")).toHaveTextContent("2");
    expect(screen.getByTitle("1 background shell running")).toHaveTextContent(
      "1",
    );
  });

  it("hides zero counters and counters outside Working", () => {
    const { rerender } = render(
      <StatusPill state="working" subagents={0} shells={0} />,
    );
    expect(screen.queryByTitle(/running$/)).toBeNull();
    rerender(<StatusPill state="idle" subagents={2} shells={1} />);
    expect(screen.queryByTitle(/running$/)).toBeNull();
  });

  it("renders the count form as glyph plus number", () => {
    render(<StatusPill state="needs" count={3} />);
    const pill = screen.getByRole("img", { name: "3 needs you" });
    expect(pill).toHaveTextContent(/^3$/);
  });
});
