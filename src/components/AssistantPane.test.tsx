import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

// Mock the IPC command layer so the store talks to a fixture, not Tauri.
vi.mock("../ipc/commands", () => ({
  assistantSend: vi.fn(() => Promise.resolve()),
  assistantAutonomyGet: vi.fn(() => Promise.resolve("confirm-destructive")),
  assistantAutonomySet: vi.fn((mode: string) => Promise.resolve(mode)),
  assistantResolveProposal: vi.fn(() => Promise.resolve()),
  assistantTranscript: vi.fn(() => Promise.resolve([])),
  assistantNewConversation: vi.fn(() => Promise.resolve()),
  assistantMemoryGet: vi.fn(() => Promise.resolve("")),
  assistantMemorySet: vi.fn(() => Promise.resolve()),
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
  assistantMemoryGet,
  assistantMemorySet,
  assistantNewConversation,
  assistantResolveProposal,
  assistantSend,
  assistantTranscript,
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
    vi.mocked(assistantTranscript).mockReset();
    vi.mocked(assistantTranscript).mockResolvedValue([]);
    vi.mocked(assistantNewConversation).mockReset();
    vi.mocked(assistantNewConversation).mockResolvedValue(undefined);
    vi.mocked(assistantMemoryGet).mockReset();
    vi.mocked(assistantMemoryGet).mockResolvedValue("");
    vi.mocked(assistantMemorySet).mockReset();
    vi.mocked(assistantMemorySet).mockResolvedValue(undefined);
    useAssistantStore.setState({
      messages: [],
      busy: false,
      activity: null,
      autonomy: "confirm-destructive",
      memory: null,
      hydrated: false,
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
    expect(screen.getByRole("status")).toHaveTextContent("thinking…");
  });

  it("grows the composer with the draft and shrinks back after send", async () => {
    // jsdom does no layout, so scrollHeight is always 0 — stub it so the
    // resize effect has something real to react to. This only proves the
    // effect runs and writes the style; the visual max-height cap (CSS
    // `max-height` + `overflow-y: auto` on `.input`) is not covered here.
    let scrollHeight = 32;
    Object.defineProperty(HTMLTextAreaElement.prototype, "scrollHeight", {
      configurable: true,
      get: () => scrollHeight,
    });
    const user = userEvent.setup();
    render(<AssistantPane />);
    const input = screen.getByLabelText(
      "Message the assistant",
    ) as HTMLTextAreaElement;

    scrollHeight = 96;
    await user.type(input, "a long draft that wraps several lines");
    expect(input.style.height).toBe("96px");

    scrollHeight = 32;
    await user.click(screen.getByLabelText("Send"));
    expect(input.style.height).toBe("32px");
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

  it("demotes a proactive nudge instead of showing it as a message to the user", () => {
    render(<AssistantPane />);

    act(() => {
      useAssistantStore.getState().apply({
        type: "notice",
        text: '[Podium] The agent "docs" in project "Elements" finished (exit code 0). Check what it did and tell the user.',
      });
    });

    // A discreet marker, not the raw internal instruction text.
    expect(
      screen.getByText("Podium nudged the assistant"),
    ).toBeInTheDocument();
    expect(screen.queryByText(/Check what it did/)).not.toBeInTheDocument();
  });

  it("shows the running tool in the busy row while it is active", async () => {
    const user = userEvent.setup();
    render(<AssistantPane />);

    await user.type(screen.getByLabelText("Message the assistant"), "hi");
    await user.click(screen.getByLabelText("Send"));
    expect(screen.getByRole("status")).toHaveTextContent("thinking…");

    act(() => {
      useAssistantStore
        .getState()
        .apply({ type: "tool", name: "mcp__podium__list_todos" });
    });
    expect(screen.getByRole("status")).toHaveTextContent("list todos…");
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
    expect(screen.getByRole("status")).toHaveTextContent("thinking…");
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

  describe("persistence and memory", () => {
    it("replays the stored transcript into the log on mount", async () => {
      vi.mocked(assistantTranscript).mockResolvedValue([
        { role: "user", text: "what is open?", at: "2026-01-02T09:00:00Z" },
        { role: "assistant", text: "Two to-dos.", at: "2026-01-02T09:00:01Z" },
        { role: "tool", text: "list_todos", at: "2026-01-02T09:00:02Z" },
      ]);

      render(<AssistantPane />);

      expect(await screen.findByText("what is open?")).toBeInTheDocument();
      expect(screen.getByText("Two to-dos.")).toBeInTheDocument();
      expect(screen.getByText("list todos")).toBeInTheDocument();
    });

    it("offers no new-conversation control — resetting lives in Settings", async () => {
      vi.mocked(assistantTranscript).mockResolvedValue([
        { role: "user", text: "old turn", at: "2026-01-02T09:00:00Z" },
      ]);
      render(<AssistantPane />);
      expect(await screen.findByText("old turn")).toBeInTheDocument();

      expect(screen.queryByLabelText("New conversation")).toBeNull();
      expect(assistantNewConversation).not.toHaveBeenCalled();
    });

    it("shows the stored memory and saves an edit", async () => {
      const user = userEvent.setup();
      vi.mocked(assistantMemoryGet).mockResolvedValue("Prefers short answers.");
      render(<AssistantPane />);

      await user.click(screen.getByLabelText("Assistant memory"));
      const box = await screen.findByLabelText("Memory");
      expect(box).toHaveValue("Prefers short answers.");

      await user.type(box, " Ships on Fridays.");
      await user.click(screen.getByRole("button", { name: "Save" }));

      expect(assistantMemorySet).toHaveBeenCalledWith(
        "Prefers short answers. Ships on Fridays.",
      );
      // Saving closes the view again.
      expect(screen.queryByLabelText("Memory")).not.toBeInTheDocument();
    });

    it("clears the memory box, then persists the clear on save", async () => {
      const user = userEvent.setup();
      vi.mocked(assistantMemoryGet).mockResolvedValue("Something old.");
      render(<AssistantPane />);

      await user.click(screen.getByLabelText("Assistant memory"));
      expect(await screen.findByLabelText("Memory")).toHaveValue(
        "Something old.",
      );

      await user.click(screen.getByRole("button", { name: "Clear" }));
      expect(screen.getByLabelText("Memory")).toHaveValue("");

      await user.click(screen.getByRole("button", { name: "Save" }));
      expect(assistantMemorySet).toHaveBeenCalledWith("");
    });
  });
});
