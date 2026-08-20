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
import { useAssistantStore } from "./assistantStore";

describe("assistantStore", () => {
  beforeEach(() => {
    vi.mocked(assistantSend).mockClear();
    useAssistantStore.setState({ messages: [], busy: false });
  });

  it("refuses a blank turn without touching the backend", async () => {
    const sent = await useAssistantStore.getState().send("   ");

    expect(sent).toBe(false);
    expect(assistantSend).not.toHaveBeenCalled();
    expect(useAssistantStore.getState().messages).toEqual([]);
  });

  it("trims the turn and stays busy until the turn ends", async () => {
    await useAssistantStore.getState().send("  hello  ");
    expect(assistantSend).toHaveBeenCalledWith("hello");
    expect(useAssistantStore.getState().busy).toBe(true);

    useAssistantStore.getState().apply({ type: "turnEnd", error: null });
    expect(useAssistantStore.getState().busy).toBe(false);
    expect(
      useAssistantStore.getState().messages.map((m) => [m.role, m.text]),
    ).toEqual([["user", "hello"]]);
  });

  it("ignores the session-id event but reports an ended session", () => {
    const { apply } = useAssistantStore.getState();
    apply({ type: "started", sessionId: "sess-1" });
    expect(useAssistantStore.getState().messages).toEqual([]);

    apply({ type: "exited" });
    const [last] = useAssistantStore.getState().messages;
    expect(last.role).toBe("notice");
    expect(last.text).toContain("session ended");
    expect(useAssistantStore.getState().busy).toBe(false);
  });
});
