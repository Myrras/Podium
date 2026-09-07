import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

// The file picker is a Tauri plugin: mocked so the picker button resolves
// to fixture paths instead of opening a real dialog.
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(() => Promise.resolve(null)),
}));

// Mock the IPC command layer so the store talks to a fixture, not Tauri.
vi.mock("../ipc/commands", () => ({
  assistantSend: vi.fn(() => Promise.resolve()),
  assistantDayStart: vi.fn(() => Promise.resolve()),
  assistantAutonomyGet: vi.fn(() => Promise.resolve("confirm-destructive")),
  assistantAutonomySet: vi.fn((mode: string) => Promise.resolve(mode)),
  assistantResolveProposal: vi.fn(() => Promise.resolve()),
  assistantTranscript: vi.fn(() => Promise.resolve([])),
  assistantNewConversation: vi.fn(() => Promise.resolve()),
  assistantMemoryGet: vi.fn(() => Promise.resolve("")),
  assistantMemorySet: vi.fn(() => Promise.resolve()),
  assistantQueueEdit: vi.fn(() => Promise.resolve()),
  assistantQueueRemove: vi.fn(() => Promise.resolve()),
  assistantInterrupt: vi.fn(() => Promise.resolve()),
  toIpcError: (e: unknown) => {
    if (typeof e === "object" && e !== null && "kind" in e && "message" in e) {
      return e as { kind: string; message: string };
    }
    return { kind: "unknown", message: String(e) };
  },
}));

import { open as openFileDialog } from "@tauri-apps/plugin-dialog";

import {
  assistantAutonomyGet,
  assistantAutonomySet,
  assistantDayStart,
  assistantInterrupt,
  assistantMemoryGet,
  assistantMemorySet,
  assistantNewConversation,
  assistantQueueEdit,
  assistantQueueRemove,
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
    vi.mocked(assistantDayStart).mockReset();
    vi.mocked(assistantDayStart).mockResolvedValue(undefined);
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
    vi.mocked(assistantQueueEdit).mockReset();
    vi.mocked(assistantQueueEdit).mockResolvedValue(undefined);
    vi.mocked(assistantQueueRemove).mockReset();
    vi.mocked(assistantQueueRemove).mockResolvedValue(undefined);
    vi.mocked(assistantInterrupt).mockReset();
    vi.mocked(assistantInterrupt).mockResolvedValue(undefined);
    vi.mocked(openFileDialog).mockReset();
    vi.mocked(openFileDialog).mockResolvedValue(null);
    useAssistantStore.setState({
      messages: [],
      busy: false,
      activity: null,
      autonomy: "confirm-destructive",
      memory: null,
      hydrated: false,
      pending: [],
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

    expect(assistantSend).toHaveBeenCalledWith("what is open?", []);
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
    expect(assistantSend).toHaveBeenCalledWith("first\nsecond", []);
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
    expect(screen.getByText("Podium nudged the assistant")).toBeInTheDocument();
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

    it("demotes a replayed nudge just like a live one", async () => {
      vi.mocked(assistantTranscript).mockResolvedValue([
        {
          role: "nudge",
          text: '[Podium] The agent "docs" finished (exit code 0). Check what it did and tell the user.',
          at: "2026-01-02T09:00:00Z",
        },
      ]);

      render(<AssistantPane />);

      expect(
        await screen.findByText("Podium nudged the assistant"),
      ).toBeInTheDocument();
      expect(screen.queryByText(/Check what it did/)).not.toBeInTheDocument();
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

  describe("the pending queue", () => {
    it("shows nothing when the queue is empty", () => {
      render(<AssistantPane />);
      expect(screen.queryByLabelText("Edit waiting message")).toBeNull();
      expect(screen.queryByLabelText("Remove waiting message")).toBeNull();
    });

    it("renders a waiting user message with edit/remove controls", () => {
      useAssistantStore.setState({
        pending: [{ id: "p-1", kind: "user", text: "queued while busy", attachments: [] }],
      });
      render(<AssistantPane />);

      expect(screen.getByText("queued while busy")).toBeInTheDocument();
      expect(screen.getByText("waiting…")).toBeInTheDocument();
      expect(screen.getByLabelText("Edit waiting message")).toBeInTheDocument();
      expect(
        screen.getByLabelText("Remove waiting message"),
      ).toBeInTheDocument();
    });

    it("does not offer edit/remove for a nudge or notice waiting its turn", () => {
      useAssistantStore.setState({
        pending: [
          { id: "n-1", kind: "nudge", text: "[Podium] an agent finished", attachments: [] },
        ],
      });
      render(<AssistantPane />);

      expect(
        screen.getByText("[Podium] an agent finished"),
      ).toBeInTheDocument();
      expect(screen.queryByLabelText("Edit waiting message")).toBeNull();
      expect(screen.queryByLabelText("Remove waiting message")).toBeNull();
    });

    it("edits a waiting message on Enter", async () => {
      const user = userEvent.setup();
      useAssistantStore.setState({
        pending: [{ id: "p-1", kind: "user", text: "first draft", attachments: [] }],
      });
      render(<AssistantPane />);

      await user.click(screen.getByLabelText("Edit waiting message"));
      const input = screen.getByLabelText("Edit waiting message");
      await user.clear(input);
      await user.type(input, "final draft{Enter}");

      expect(assistantQueueEdit).toHaveBeenCalledWith("p-1", "final draft");
    });

    it("removes a waiting message", async () => {
      const user = userEvent.setup();
      useAssistantStore.setState({
        pending: [{ id: "p-1", kind: "user", text: "withdraw me", attachments: [] }],
      });
      render(<AssistantPane />);

      await user.click(screen.getByLabelText("Remove waiting message"));

      expect(assistantQueueRemove).toHaveBeenCalledWith("p-1");
    });

    it("offers an interrupt control only while a turn is running", async () => {
      const user = userEvent.setup();
      const { rerender } = render(<AssistantPane />);
      expect(screen.queryByLabelText("Interrupt")).toBeNull();

      useAssistantStore.setState({ busy: true });
      rerender(<AssistantPane />);
      await user.click(screen.getByLabelText("Interrupt"));

      expect(assistantInterrupt).toHaveBeenCalled();
    });
  });

  describe("composer history recall", () => {
    function seedSentMessages() {
      useAssistantStore.setState({
        messages: [
          { id: 1, role: "user", text: "first message" },
          { id: 2, role: "assistant", text: "an answer" },
          { id: 3, role: "user", text: "second message" },
        ],
      });
    }

    it("arrow-up recalls the previous sent messages, newest first", async () => {
      seedSentMessages();
      const user = userEvent.setup();
      render(<AssistantPane />);
      const input = screen.getByLabelText(
        "Message the assistant",
      ) as HTMLTextAreaElement;
      input.focus();

      await user.keyboard("{ArrowUp}");
      expect(input).toHaveValue("second message");

      await user.keyboard("{ArrowUp}");
      expect(input).toHaveValue("first message");

      // Already at the oldest entry — stays put rather than wrapping.
      await user.keyboard("{ArrowUp}");
      expect(input).toHaveValue("first message");
    });

    it("arrow-down walks forward and restores the draft past the newest entry", async () => {
      seedSentMessages();
      const user = userEvent.setup();
      render(<AssistantPane />);
      const input = screen.getByLabelText(
        "Message the assistant",
      ) as HTMLTextAreaElement;
      input.focus();
      await user.type(input, "my unsent draft");

      await user.keyboard("{ArrowUp}");
      expect(input).toHaveValue("second message");
      await user.keyboard("{ArrowUp}");
      expect(input).toHaveValue("first message");

      await user.keyboard("{ArrowDown}");
      expect(input).toHaveValue("second message");

      // Past the newest recalled entry: back to what was being typed.
      await user.keyboard("{ArrowDown}");
      expect(input).toHaveValue("my unsent draft");
    });

    it("does not recall when the caret isn't on the first/last line", async () => {
      seedSentMessages();
      const user = userEvent.setup();
      render(<AssistantPane />);
      const input = screen.getByLabelText(
        "Message the assistant",
      ) as HTMLTextAreaElement;

      await user.type(input, "line one{Shift>}{Enter}{/Shift}line two");
      // Caret sits at the end, on the second line — arrow-up should move the
      // cursor within the text, not recall history.
      await user.keyboard("{ArrowUp}");
      expect(input).toHaveValue("line one\nline two");

      // Move the caret to the very start (first line) and try again: now it
      // recalls.
      input.setSelectionRange(0, 0);
      await user.keyboard("{ArrowUp}");
      expect(input).toHaveValue("second message");
    });

    it("does not send while recalling, and sending a recalled+edited message leaves history intact", async () => {
      seedSentMessages();
      const user = userEvent.setup();
      render(<AssistantPane />);
      const input = screen.getByLabelText(
        "Message the assistant",
      ) as HTMLTextAreaElement;
      input.focus();

      await user.keyboard("{ArrowUp}");
      expect(assistantSend).not.toHaveBeenCalled();
      expect(input).toHaveValue("second message");

      await user.type(input, " edited");
      await user.click(screen.getByLabelText("Send"));
      expect(assistantSend).toHaveBeenCalledWith("second message edited", []);
      // Sending moved focus to the Send button; bring it back to the
      // composer before recalling again.
      input.focus();

      // Recalling again finds the sent message as a new, separate entry —
      // the original "second message" is still there behind it.
      await user.keyboard("{ArrowUp}");
      expect(input).toHaveValue("second message edited");
      await user.keyboard("{ArrowUp}");
      expect(input).toHaveValue("second message");
    });

    it("does nothing when there is no sent history", async () => {
      const user = userEvent.setup();
      render(<AssistantPane />);
      const input = screen.getByLabelText(
        "Message the assistant",
      ) as HTMLTextAreaElement;
      input.focus();
      await user.type(input, "typing");

      await user.keyboard("{ArrowUp}");
      expect(input).toHaveValue("typing");
    });
  });

  describe("day start", () => {
    it("shows the day-start button in the empty state and triggers it", async () => {
      const user = userEvent.setup();
      render(<AssistantPane />);

      await user.click(screen.getByLabelText("Start my day"));

      expect(assistantDayStart).toHaveBeenCalled();
      // The turn shows at once, so the button is gone before the CLI answers
      // — no window in which the click looks like it did nothing.
      expect(screen.queryByLabelText("Start my day")).toBeNull();
      expect(screen.getByText("Start my day")).toBeInTheDocument();
      expect(screen.getByText("thinking…")).toBeInTheDocument();
    });

    it("hides the day-start button once there are messages", () => {
      useAssistantStore.setState({
        messages: [{ id: 1, role: "user", text: "hi" }],
      });
      render(<AssistantPane />);

      expect(screen.queryByLabelText("Start my day")).toBeNull();
    });
  });

  describe("attachments", () => {
    it("attaches picked files as absolute paths and sends them with the turn", async () => {
      const user = userEvent.setup();
      vi.mocked(openFileDialog).mockResolvedValue([
        "/abs/notes.txt",
      ] as unknown as string);
      render(<AssistantPane />);

      await user.click(screen.getByLabelText("Attach files"));
      expect(screen.getByText("notes.txt")).toBeInTheDocument();

      await user.type(screen.getByLabelText("Message the assistant"), "read it");
      await user.click(screen.getByLabelText("Send"));

      expect(assistantSend).toHaveBeenCalledWith("read it", [
        { kind: "path", path: "/abs/notes.txt" },
      ]);
      // The tray clears with the composer.
      expect(screen.queryByText("notes.txt")).toBeNull();
    });

    it("removes an attachment before it is sent", async () => {
      const user = userEvent.setup();
      vi.mocked(openFileDialog).mockResolvedValue([
        "/abs/notes.txt",
      ] as unknown as string);
      render(<AssistantPane />);

      await user.click(screen.getByLabelText("Attach files"));
      await user.click(screen.getByLabelText("Remove notes.txt"));

      expect(screen.queryByText("notes.txt")).toBeNull();
      // Nothing left to send, so the button goes back to disabled.
      expect(screen.getByLabelText("Send")).toBeDisabled();
    });

    it("sends an attachment with no typed message", async () => {
      const user = userEvent.setup();
      vi.mocked(openFileDialog).mockResolvedValue([
        "/abs/shot.png",
      ] as unknown as string);
      render(<AssistantPane />);

      await user.click(screen.getByLabelText("Attach files"));
      expect(screen.getByLabelText("Send")).toBeEnabled();
      await user.click(screen.getByLabelText("Send"));

      expect(assistantSend).toHaveBeenCalledWith("(sent without a message)", [
        { kind: "path", path: "/abs/shot.png" },
      ]);
    });

    it("refuses more than five attachments, naming each one it dropped", async () => {
      const user = userEvent.setup();
      vi.mocked(openFileDialog).mockResolvedValue(
        Array.from({ length: 7 }, (_, i) => `/abs/f${i}.txt`) as unknown as string,
      );
      render(<AssistantPane />);

      await user.click(screen.getByLabelText("Attach files"));

      expect(screen.getByText("f4.txt")).toBeInTheDocument();
      expect(screen.queryByText("f5.txt")).toBeNull();
      // The two over the cap are reported, not silently dropped.
      expect(
        screen.getByText(/"f5.txt" was not added/),
      ).toBeInTheDocument();
      expect(
        screen.getByText(/"f6.txt" was not added/),
      ).toBeInTheDocument();
    });

    it("attaches a pasted image as inline base64", async () => {
      render(<AssistantPane />);
      const input = screen.getByLabelText("Message the assistant");
      const file = new File([new Uint8Array([1, 2, 3])], "shot.png", {
        type: "image/png",
      });

      await act(async () => {
        fireEvent.paste(input, { clipboardData: { files: [file] } });
      });

      expect(await screen.findByText("shot.png")).toBeInTheDocument();
      await act(async () => {
        fireEvent.click(screen.getByLabelText("Send"));
      });
      expect(assistantSend).toHaveBeenCalledWith("(sent without a message)", [
        { kind: "image", name: "shot.png", mediaType: "image/png", data: "AQID" },
      ]);
    });

    it("attaches a dropped text file inline, and reports one it cannot take", async () => {
      render(<AssistantPane />);
      const pane = screen.getByLabelText("Personal assistant");
      const text = new File(["log line"], "run.log", { type: "text/plain" });
      const binary = new File([new Uint8Array([0, 1])], "archive.zip", {
        type: "application/zip",
      });

      await act(async () => {
        fireEvent.drop(pane, { dataTransfer: { files: [text, binary] } });
      });

      expect(await screen.findByText("run.log")).toBeInTheDocument();
      // A dropped file carries bytes but no path, so a binary one has no way
      // through — it is named rather than vanishing.
      expect(
        screen.getByText(/"archive.zip" was not added/),
      ).toBeInTheDocument();
    });
  });
});
