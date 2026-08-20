import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

// Mock the IPC command layer so the store talks to a fixture, not Tauri.
vi.mock("../ipc/commands", () => ({
  assistantSend: vi.fn(() => Promise.resolve()),
  toIpcError: (e: unknown) => {
    if (typeof e === "object" && e !== null && "kind" in e && "message" in e) {
      return e as { kind: string; message: string };
    }
    return { kind: "unknown", message: String(e) };
  },
}));

import { assistantSend } from "../ipc/commands";
import { useAssistantStore } from "../state/assistantStore";
import { useLayoutStore } from "../state/layoutStore";
import { AssistantPane } from "./AssistantPane";

describe("AssistantPane", () => {
  beforeEach(() => {
    vi.mocked(assistantSend).mockReset();
    vi.mocked(assistantSend).mockResolvedValue(undefined);
    useAssistantStore.setState({ messages: [], busy: false });
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

  it("renders streamed prose, tool calls and failures", () => {
    render(<AssistantPane />);

    act(() => {
      const { apply } = useAssistantStore.getState();
      apply({ type: "message", text: "**Two** agents are running" });
      apply({ type: "tool", name: "list_processes" });
      apply({ type: "turnEnd", error: "error_max_turns" });
    });

    expect(screen.getByText("Two")).toBeInTheDocument();
    expect(screen.getByText("list_processes")).toBeInTheDocument();
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

  it("hides itself via the header button", async () => {
    const user = userEvent.setup();
    useLayoutStore.setState({ assistantOpen: true });
    render(<AssistantPane />);

    await user.click(screen.getByLabelText("Hide assistant"));

    expect(useLayoutStore.getState().assistantOpen).toBe(false);
  });
});
