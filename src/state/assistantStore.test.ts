import { beforeEach, describe, expect, it, vi } from "vitest";

// Mock the IPC command layer so the store talks to a fixture, not Tauri.
vi.mock("../ipc/commands", () => ({
  assistantSend: vi.fn(() => Promise.resolve()),
  assistantProactivityGet: vi.fn(() => Promise.resolve(true)),
  assistantProactivitySet: vi.fn((enabled: boolean) =>
    Promise.resolve(enabled),
  ),
  toIpcError: (e: unknown) => {
    if (typeof e === "object" && e !== null && "kind" in e && "message" in e) {
      return e as { kind: string; message: string };
    }
    return { kind: "unknown", message: String(e) };
  },
}));

import {
  assistantProactivityGet,
  assistantProactivitySet,
  assistantSend,
} from "../ipc/commands";
import { useAssistantStore } from "./assistantStore";

describe("assistantStore", () => {
  beforeEach(() => {
    vi.mocked(assistantSend).mockClear();
    vi.mocked(assistantProactivityGet).mockClear();
    vi.mocked(assistantProactivitySet).mockClear();
    vi.mocked(assistantProactivitySet).mockImplementation((enabled) =>
      Promise.resolve(enabled),
    );
    useAssistantStore.setState({ messages: [], busy: false, proactive: true });
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

  it("shows a proactive nudge as Podium speaking in the log", () => {
    useAssistantStore
      .getState()
      .apply({ type: "notice", text: '[Podium] The agent "docs" finished.' });

    const [row] = useAssistantStore.getState().messages;
    expect(row.role).toBe("notice");
    expect(row.text).toContain("finished");
  });

  it("mirrors the proactivity setting and rolls back a failed save", async () => {
    await useAssistantStore.getState().loadProactivity();
    expect(assistantProactivityGet).toHaveBeenCalled();
    expect(useAssistantStore.getState().proactive).toBe(true);

    await useAssistantStore.getState().setProactivity(false);
    expect(assistantProactivitySet).toHaveBeenCalledWith(false);
    expect(useAssistantStore.getState().proactive).toBe(false);

    vi.mocked(assistantProactivitySet).mockRejectedValueOnce({
      kind: "io",
      message: "disk is full",
    });
    await useAssistantStore.getState().setProactivity(true);
    expect(useAssistantStore.getState().proactive).toBe(false);
    const msgs = useAssistantStore.getState().messages;
    const last = msgs[msgs.length - 1];
    expect(last?.text).toBe("disk is full");
  });
});
