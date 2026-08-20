import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

// Mock the IPC command layer so the store talks to a fixture, not Tauri.
vi.mock("../ipc/commands", () => ({
  assistantSend: vi.fn(() => Promise.resolve()),
  assistantAutonomyGet: vi.fn(() => Promise.resolve("confirm-destructive")),
  assistantAutonomySet: vi.fn((mode: string) => Promise.resolve(mode)),
  assistantResolveProposal: vi.fn(() => Promise.resolve()),
  toIpcError: (e: unknown) => {
    if (typeof e === "object" && e !== null && "kind" in e && "message" in e) {
      return e as { kind: string; message: string };
    }
    return { kind: "unknown", message: String(e) };
  },
}));

import {
  assistantAutonomyGet,
  assistantAutonomySet,
  assistantResolveProposal,
  assistantSend,
} from "../ipc/commands";
import { useAssistantStore } from "../state/assistantStore";
import { useLayoutStore } from "../state/layoutStore";
import { AssistantPane } from "./AssistantPane";

describe("AssistantPane", () => {
  beforeEach(() => {
    vi.mocked(assistantSend).mockReset();
    vi.mocked(assistantSend).mockResolvedValue(undefined);
    vi.mocked(assistantAutonomyGet).mockReset();
    vi.mocked(assistantAutonomyGet).mockResolvedValue("confirm-destructive");
    vi.mocked(assistantAutonomySet).mockReset();
    vi.mocked(assistantAutonomySet).mockImplementation((mode) =>
      Promise.resolve(mode),
    );
    vi.mocked(assistantResolveProposal).mockReset();
    vi.mocked(assistantResolveProposal).mockResolvedValue(undefined);
    useAssistantStore.setState({
      messages: [],
      busy: false,
      autonomy: "confirm-destructive",
    });
  });

  it("sends the typed turn and shows it in the log", async () => {
    const user = userEvent.setup();
    render(<AssistantPane />);

    await user.type(
      screen.getByLabelText("Message the assistant"),
      "what is open?",
    );
    await user.click(screen.getByLabelText("Send"));

    expect(assistantSend).toHaveBeenCalledWith("what is open?");
    expect(screen.getByText("what is open?")).toBeInTheDocument();
    // The composer clears so the next turn starts empty.
    expect(screen.getByLabelText("Message the assistant")).toHaveValue("");
    expect(screen.getByRole("status")).toHaveTextContent("working…");
  });

  it("sends on Enter but not on Shift+Enter", async () => {
    const user = userEvent.setup();
    render(<AssistantPane />);
    const input = screen.getByLabelText("Message the assistant");

    await user.type(input, "first{Shift>}{Enter}{/Shift}second");
    expect(assistantSend).not.toHaveBeenCalled();

    await user.type(input, "{Enter}");
    expect(assistantSend).toHaveBeenCalledWith("first\nsecond");
  });

  it("renders streamed prose, executed actions and failures", () => {
    render(<AssistantPane />);

    act(() => {
      const { apply } = useAssistantStore.getState();
      apply({ type: "message", text: "**Two** agents are running" });
      apply({ type: "tool", name: "mcp__podium__list_processes" });
      apply({ type: "toolFailed", message: "to-do not found" });
      apply({ type: "turnEnd", error: "error_max_turns" });
    });

    expect(screen.getByText("Two")).toBeInTheDocument();
    // The action card names the action, without its MCP plumbing.
    expect(screen.getByText("list processes")).toBeInTheDocument();
    expect(
      screen.getByText("That action failed: to-do not found"),
    ).toBeInTheDocument();
    expect(
      screen.getByText("The turn failed: error_max_turns"),
    ).toBeInTheDocument();
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("reports a failed send in the log instead of losing it", async () => {
    vi.mocked(assistantSend).mockRejectedValueOnce({
      kind: "invalidInput",
      message: "message must not be blank",
    });
    const user = userEvent.setup();
    render(<AssistantPane />);

    await user.type(screen.getByLabelText("Message the assistant"), "hi");
    await user.click(screen.getByLabelText("Send"));

    expect(screen.getByText("message must not be blank")).toBeInTheDocument();
  });

  /** Stream in one pending proposal card. */
  function streamProposal() {
    act(() => {
      useAssistantStore.getState().apply({
        type: "proposal",
        id: "p-1",
        tool: "set_todo_archived",
        summary: '{"todoId":"t-1"}',
      });
    });
  }

  it("confirms a proposed action", async () => {
    const user = userEvent.setup();
    render(<AssistantPane />);
    streamProposal();

    expect(screen.getByText("set todo archived")).toBeInTheDocument();
    expect(screen.getByText('{"todoId":"t-1"}')).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Confirm" }));

    expect(assistantResolveProposal).toHaveBeenCalledWith("p-1", true);
    // The buttons are gone: one decision per proposal.
    expect(
      screen.queryByRole("button", { name: "Confirm" }),
    ).not.toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("working…");
  });

  it("rejects a proposed action and shows the outcome", async () => {
    const user = userEvent.setup();
    render(<AssistantPane />);
    streamProposal();

    await user.click(screen.getByRole("button", { name: "Reject" }));

    expect(assistantResolveProposal).toHaveBeenCalledWith("p-1", false);
    expect(screen.getByText("Rejected")).toBeInTheDocument();
  });

  it("reports a confirmed action that failed", () => {
    render(<AssistantPane />);
    streamProposal();

    act(() => {
      useAssistantStore.getState().apply({
        type: "proposalResolved",
        id: "p-1",
        approved: true,
        error: "to-do not found",
      });
    });

    expect(screen.getByText("Failed: to-do not found")).toBeInTheDocument();
  });

  it("loads and changes the autonomy mode", async () => {
    vi.mocked(assistantAutonomyGet).mockResolvedValue("propose-all");
    const user = userEvent.setup();
    render(<AssistantPane />);

    await vi.waitFor(() =>
      expect(screen.getByLabelText("Autonomy")).toHaveValue("propose-all"),
    );

    await user.selectOptions(screen.getByLabelText("Autonomy"), "full");

    expect(assistantAutonomySet).toHaveBeenCalledWith("full");
    expect(useAssistantStore.getState().autonomy).toBe("full");
  });

  it("hides itself via the header button", async () => {
    const user = userEvent.setup();
    useLayoutStore.setState({ assistantOpen: true });
    render(<AssistantPane />);

    await user.click(screen.getByLabelText("Hide assistant"));

    expect(useLayoutStore.getState().assistantOpen).toBe(false);
  });
});
