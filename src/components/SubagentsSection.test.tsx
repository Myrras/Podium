import { fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// Mock the IPC command layer so the store talks to a fixture, not Tauri.
vi.mock("../ipc/commands", () => ({
  assistantSubagentStop: vi.fn(() => Promise.resolve()),
  toIpcError: (e: unknown) => ({ kind: "unknown", message: String(e) }),
}));

import { assistantSubagentStop } from "../ipc/commands";
import type { SubagentInfo } from "../ipc/types";
import { useAssistantStore } from "../state/assistantStore";
import { useLayoutStore } from "../state/layoutStore";
import { useProcessStore } from "../state/processStore";
import { SubagentsSection } from "./SubagentsSection";

const initialAssistant = useAssistantStore.getState();
const initialLayout = useLayoutStore.getState();

function subagent(overrides: Partial<SubagentInfo> = {}): SubagentInfo {
  return {
    id: "sub-1",
    name: "Ticket check",
    kind: "lookup",
    task: "Check the open support tickets for the Webshop release",
    phase: "running",
    status: { state: "running", pid: 7, since: "2026-01-01T10:00:00Z" },
    createdAt: "2026-01-01T10:00:00Z",
    startedAt: "2026-01-01T10:00:00Z",
    endedAt: null,
    expiresAt: null,
    result: null,
    error: null,
    question: null,
    ...overrides,
  };
}

function seed(subagents: SubagentInfo[]) {
  useAssistantStore.setState({ ...initialAssistant, subagents }, true);
}

describe("SubagentsSection", () => {
  beforeEach(() => {
    vi.mocked(assistantSubagentStop).mockClear();
    useLayoutStore.setState(initialLayout, true);
    // Only `Date` is faked, so the running times are fixed.
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date("2026-01-01T10:01:05Z"));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("is hidden when there are no sub-agents", () => {
    seed([]);
    const { container } = render(<SubagentsSection />);
    expect(container).toBeEmptyDOMElement();
  });

  it("lists each sub-agent with its kind, phase and time, and counts the live ones", () => {
    seed([
      subagent(),
      subagent({
        id: "sub-2",
        name: "Repo sweep",
        kind: "full",
        task: "Repo sweep",
        phase: "queued",
        status: { state: "notStarted" },
        startedAt: null,
      }),
      subagent({
        id: "sub-3",
        name: "Release notes",
        task: "Release notes",
        phase: "done",
        status: {
          state: "exited",
          code: 0,
          crashed: false,
          stopped: true,
          at: "2026-01-01T10:00:42Z",
        },
        endedAt: "2026-01-01T10:00:42Z",
        result: "Notes drafted in the Webshop scratchpad.",
      }),
      subagent({
        id: "sub-4",
        name: "Invoice lookup",
        task: "Invoice lookup",
        phase: "failed",
        endedAt: "2026-01-01T10:00:03Z",
        error: "exited without a result",
      }),
    ]);
    render(<SubagentsSection />);

    expect(screen.getByText("1 running · 1 queued")).toBeInTheDocument();
    const rows = screen.getAllByRole("listitem");
    expect(rows).toHaveLength(4);

    const running = within(rows[0]);
    expect(running.getByText("Ticket check")).toHaveAttribute(
      "title",
      "Check the open support tickets for the Webshop release",
    );
    expect(running.getByText("lookup")).toBeInTheDocument();
    expect(running.getByText("Running")).toBeInTheDocument();
    expect(running.getByText("1m 05s")).toBeInTheDocument();
    // The task differs from the name, so it gets its own line too.
    expect(
      running.getByText(
        "Check the open support tickets for the Webshop release",
      ),
    ).toBeInTheDocument();

    const queued = within(rows[1]);
    expect(queued.getByText("full")).toBeInTheDocument();
    expect(queued.getByText("Queued")).toBeInTheDocument();
    expect(queued.getByText("queued")).toBeInTheDocument();
    // Nothing to open before it started; it can still be cancelled.
    expect(queued.queryByLabelText("Open Repo sweep")).toBeNull();
    expect(queued.getByLabelText("Stop Repo sweep")).toBeInTheDocument();

    const done = within(rows[2]);
    expect(done.getByText("Done")).toBeInTheDocument();
    expect(done.getByText("42s")).toBeInTheDocument();
    // The result is a tooltip on the phase, never a line of the row.
    expect(done.getByText("Done")).toHaveAttribute(
      "title",
      "Notes drafted in the Webshop scratchpad.",
    );
    expect(
      done.queryByText("Notes drafted in the Webshop scratchpad."),
    ).toBeNull();
    // Over: no stop, but its terminal can still be read.
    expect(done.queryByLabelText("Stop Release notes")).toBeNull();
    expect(done.getByLabelText("Open Release notes")).toBeInTheDocument();

    const failed = within(rows[3]);
    expect(failed.getByText("Failed")).toBeInTheDocument();
    expect(failed.getByText("Failed")).toHaveAttribute(
      "title",
      "exited without a result",
    );
    expect(failed.queryByText("exited without a result")).toBeNull();
  });

  it("stops a sub-agent straight away, without a confirm", () => {
    seed([subagent()]);
    render(<SubagentsSection />);

    fireEvent.click(screen.getByLabelText("Stop Ticket check"));
    expect(assistantSubagentStop).toHaveBeenCalledWith("sub-1");
  });

  it("opens a sub-agent's terminal when its row is clicked", () => {
    useLayoutStore.setState({ openTodo: { projectId: "p1", todoId: "t1" } });
    useProcessStore.setState({ activeProcessId: "proc-9" });
    seed([subagent()]);
    render(<SubagentsSection />);

    // No separate open button: the row (name and task) is the target.
    expect(screen.queryAllByRole("button", { name: /^Open/ })).toHaveLength(1);
    fireEvent.click(screen.getByText("Ticket check"));
    expect(useLayoutStore.getState().openSubagent).toBe("sub-1");
    // It takes over the work area from whatever was there.
    expect(useLayoutStore.getState().openTodo).toBeNull();
    expect(useProcessStore.getState().activeProcessId).toBeNull();
  });

  it("opens from the keyboard with Enter or Space", () => {
    seed([subagent()]);
    render(<SubagentsSection />);
    const row = screen.getByRole("button", { name: "Open Ticket check" });
    expect(row).toHaveAttribute("tabindex", "0");

    fireEvent.keyDown(row, { key: "Enter" });
    expect(useLayoutStore.getState().openSubagent).toBe("sub-1");
    useLayoutStore.setState({ openSubagent: null });
    fireEvent.keyDown(row, { key: " " });
    expect(useLayoutStore.getState().openSubagent).toBe("sub-1");
    useLayoutStore.setState({ openSubagent: null });
    fireEvent.keyDown(row, { key: "a" });
    expect(useLayoutStore.getState().openSubagent).toBeNull();
  });

  it("stops without opening, and a queued row opens nothing", () => {
    seed([
      subagent(),
      subagent({
        id: "sub-2",
        name: "Repo sweep",
        task: "Repo sweep",
        phase: "queued",
        status: { state: "notStarted" },
        startedAt: null,
      }),
    ]);
    render(<SubagentsSection />);

    fireEvent.click(screen.getByLabelText("Stop Ticket check"));
    expect(assistantSubagentStop).toHaveBeenCalledWith("sub-1");
    expect(useLayoutStore.getState().openSubagent).toBeNull();

    fireEvent.click(screen.getByText("Repo sweep"));
    expect(useLayoutStore.getState().openSubagent).toBeNull();
    expect(
      screen.queryByRole("button", { name: "Open Repo sweep" }),
    ).toBeNull();
  });

  it("shows a waiting one as needing input, with its question on hover", () => {
    seed([
      subagent({
        phase: "waiting",
        question: "Close the stale tickets too, or only list them?",
      }),
    ]);
    render(<SubagentsSection />);

    expect(screen.getByText("1 needs input")).toBeInTheDocument();
    const phase = screen.getByText("Needs input");
    expect(phase).toHaveAttribute("data-phase", "waiting");
    expect(phase).toHaveAttribute(
      "title",
      "Close the stale tickets too, or only list them?",
    );
    // The question is not a line of the row.
    expect(
      screen.queryByText("Close the stale tickets too, or only list them?"),
    ).toBeNull();
    // Still live: it can be stopped, and its terminal opened to answer it.
    expect(screen.getByLabelText("Stop Ticket check")).toBeInTheDocument();
    fireEvent.click(screen.getByText("Ticket check"));
    expect(useLayoutStore.getState().openSubagent).toBe("sub-1");
  });

  it("collapses to just its header", () => {
    seed([subagent()]);
    render(<SubagentsSection />);

    fireEvent.click(screen.getByRole("button", { name: /Sub-agents/ }));
    expect(screen.queryByRole("listitem")).toBeNull();
    expect(screen.getByText("1 running")).toBeInTheDocument();
  });
});
