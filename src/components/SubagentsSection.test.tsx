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
    result: null,
    error: null,
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
    expect(
      done.getByText("Notes drafted in the Webshop scratchpad."),
    ).toBeInTheDocument();
    // Over: no stop, but its terminal can still be read.
    expect(done.queryByLabelText("Stop Release notes")).toBeNull();
    expect(done.getByLabelText("Open Release notes")).toBeInTheDocument();

    const failed = within(rows[3]);
    expect(failed.getByText("Failed")).toBeInTheDocument();
    expect(failed.getByText("exited without a result")).toBeInTheDocument();
  });

  it("stops a sub-agent straight away, without a confirm", () => {
    seed([subagent()]);
    render(<SubagentsSection />);

    fireEvent.click(screen.getByLabelText("Stop Ticket check"));
    expect(assistantSubagentStop).toHaveBeenCalledWith("sub-1");
  });

  it("opens a sub-agent's terminal in the work area", () => {
    useLayoutStore.setState({ openTodo: { projectId: "p1", todoId: "t1" } });
    useProcessStore.setState({ activeProcessId: "proc-9" });
    seed([subagent()]);
    render(<SubagentsSection />);

    fireEvent.click(screen.getByLabelText("Open Ticket check"));
    expect(useLayoutStore.getState().openSubagent).toBe("sub-1");
    // It takes over the work area from whatever was there.
    expect(useLayoutStore.getState().openTodo).toBeNull();
    expect(useProcessStore.getState().activeProcessId).toBeNull();
  });

  it("collapses to just its header", () => {
    seed([subagent()]);
    render(<SubagentsSection />);

    fireEvent.click(screen.getByRole("button", { name: /Sub-agents/ }));
    expect(screen.queryByRole("listitem")).toBeNull();
    expect(screen.getByText("1 running")).toBeInTheDocument();
  });
});
