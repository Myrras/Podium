import { beforeEach, describe, expect, it, vi } from "vitest";

// Mock the IPC command layer so the store talks to a fixture, not Tauri.
vi.mock("../ipc/commands", () => ({
  assistantSend: vi.fn(() => Promise.resolve(true)),
  assistantDayStart: vi.fn(() => Promise.resolve(true)),
  assistantProactivityGet: vi.fn(() => Promise.resolve(true)),
  assistantProactivitySet: vi.fn((enabled: boolean) =>
    Promise.resolve(enabled),
  ),
  assistantPollingGet: vi.fn(() =>
    Promise.resolve({ intervalMinutes: 10, slack: true, teams: true }),
  ),
  assistantPollingSet: vi.fn(
    (intervalMinutes: number, slack: boolean, teams: boolean) =>
      Promise.resolve({ intervalMinutes, slack, teams }),
  ),
  assistantQueueEdit: vi.fn(() => Promise.resolve()),
  assistantQueueSendNow: vi.fn(() => Promise.resolve()),
  assistantQueueRemove: vi.fn(() => Promise.resolve()),
  assistantInterrupt: vi.fn(() => Promise.resolve()),
  assistantNewConversation: vi.fn(() => Promise.resolve()),
  assistantTranscript: vi.fn(() => Promise.resolve([])),
  toIpcError: (e: unknown) => {
    if (typeof e === "object" && e !== null && "kind" in e && "message" in e) {
      return e as { kind: string; message: string };
    }
    return { kind: "unknown", message: String(e) };
  },
}));

import {
  assistantDayStart,
  assistantInterrupt,
  assistantPollingGet,
  assistantPollingSet,
  assistantProactivityGet,
  assistantProactivitySet,
  assistantQueueEdit,
  assistantQueueRemove,
  assistantQueueSendNow,
  assistantSend,
} from "../ipc/commands";

const notifyAssistant = vi.fn();
vi.mock("../lib/notify", () => ({
  notifyAssistant: (title: string, body: string) =>
    notifyAssistant(title, body),
}));

import { useLayoutStore } from "./layoutStore";
import { useAssistantStore } from "./assistantStore";

describe("assistantStore", () => {
  beforeEach(() => {
    notifyAssistant.mockClear();
    useLayoutStore.setState({ assistantOpen: false });
    vi.spyOn(document, "hasFocus").mockReturnValue(false);
    vi.mocked(assistantSend).mockClear();
    vi.mocked(assistantSend).mockResolvedValue(true);
    vi.mocked(assistantDayStart).mockClear();
    vi.mocked(assistantDayStart).mockResolvedValue(true);
    vi.mocked(assistantProactivityGet).mockClear();
    vi.mocked(assistantProactivitySet).mockClear();
    vi.mocked(assistantProactivitySet).mockImplementation((enabled) =>
      Promise.resolve(enabled),
    );
    vi.mocked(assistantPollingGet).mockClear();
    vi.mocked(assistantPollingSet).mockClear();
    vi.mocked(assistantPollingSet).mockImplementation(
      (intervalMinutes, slack, teams) =>
        Promise.resolve({ intervalMinutes, slack, teams }),
    );
    vi.mocked(assistantQueueEdit).mockClear();
    vi.mocked(assistantQueueEdit).mockResolvedValue(undefined);
    vi.mocked(assistantQueueRemove).mockClear();
    vi.mocked(assistantQueueRemove).mockResolvedValue(undefined);
    vi.mocked(assistantQueueSendNow).mockClear();
    vi.mocked(assistantQueueSendNow).mockResolvedValue(undefined);
    vi.mocked(assistantInterrupt).mockClear();
    vi.mocked(assistantInterrupt).mockResolvedValue(undefined);
    useAssistantStore.setState({
      messages: [],
      busy: false,
      activity: null,
      proactive: true,
      polling: { intervalMinutes: 10, slack: true, teams: true },
      pending: [],
      turnReplyText: null,
    });
  });

  it("refuses a blank turn without touching the backend", async () => {
    const sent = await useAssistantStore.getState().send("   ");

    expect(sent).toBe(false);
    expect(assistantSend).not.toHaveBeenCalled();
    expect(useAssistantStore.getState().messages).toEqual([]);
  });

  it("trims the turn and stays busy until the turn ends", async () => {
    await useAssistantStore.getState().send("  hello  ");
    expect(assistantSend).toHaveBeenCalledWith("hello", []);
    expect(useAssistantStore.getState().busy).toBe(true);

    useAssistantStore
      .getState()
      .apply({ type: "turnEnd", error: null, notify: true });
    expect(useAssistantStore.getState().busy).toBe(false);
    expect(
      useAssistantStore.getState().messages.map((m) => [m.role, m.text]),
    ).toEqual([["user", "hello"]]);
  });

  it("shows the day-start turn as soon as the backend takes it", async () => {
    await useAssistantStore.getState().dayStart();

    expect(assistantDayStart).toHaveBeenCalled();
    expect(
      useAssistantStore.getState().messages.map((m) => [m.role, m.text]),
    ).toEqual([["user", "Start my day"]]);
    expect(useAssistantStore.getState().busy).toBe(true);
  });

  it("shows no day-start bubble when the backend queued it instead", async () => {
    vi.mocked(assistantDayStart).mockResolvedValueOnce(false);
    await useAssistantStore.getState().dayStart();

    // It is waiting behind a running turn, and the waiting row shows it.
    expect(useAssistantStore.getState().messages).toEqual([]);
    expect(useAssistantStore.getState().busy).toBe(false);
  });

  it("reports a failed day-start trigger as a notice", async () => {
    vi.mocked(assistantDayStart).mockRejectedValueOnce({
      kind: "io",
      message: "could not spawn the session",
    });
    await useAssistantStore.getState().dayStart();

    const msgs = useAssistantStore.getState().messages;
    expect(msgs[msgs.length - 1]).toMatchObject({
      role: "notice",
      text: "could not spawn the session",
    });
    // The optimistic bubble must not leave the panel stuck on "thinking…".
    expect(useAssistantStore.getState().busy).toBe(false);
  });

  it("tracks the running tool as activity, cleared at turn end", () => {
    const { apply } = useAssistantStore.getState();
    apply({ type: "tool", name: "mcp__podium__list_todos" });
    expect(useAssistantStore.getState().activity).toBe(
      "mcp__podium__list_todos",
    );

    apply({ type: "turnEnd", error: null, notify: true });
    expect(useAssistantStore.getState().activity).toBeNull();
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

  it("keeps a proactive nudge out of the visible notice role", () => {
    useAssistantStore
      .getState()
      .apply({ type: "notice", text: '[Podium] The agent "docs" finished.' });

    const [row] = useAssistantStore.getState().messages;
    // "nudge", not "notice": it's internal instruction text for the
    // assistant, not a message written to the user (see AssistantPane).
    expect(row.role).toBe("nudge");
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

  it("loads the polling settings and rolls back a failed save", async () => {
    await useAssistantStore.getState().loadPolling();
    expect(assistantPollingGet).toHaveBeenCalled();
    expect(useAssistantStore.getState().polling).toEqual({
      intervalMinutes: 10,
      slack: true,
      teams: true,
    });

    await useAssistantStore
      .getState()
      .setPolling({ intervalMinutes: 30, slack: false, teams: true });
    expect(assistantPollingSet).toHaveBeenCalledWith(30, false, true);
    expect(useAssistantStore.getState().polling).toEqual({
      intervalMinutes: 30,
      slack: false,
      teams: true,
    });

    vi.mocked(assistantPollingSet).mockRejectedValueOnce({
      kind: "io",
      message: "disk is full",
    });
    await useAssistantStore
      .getState()
      .setPolling({ intervalMinutes: 0, slack: false, teams: false });
    // Rolled back to what the backend last confirmed.
    expect(useAssistantStore.getState().polling).toEqual({
      intervalMinutes: 30,
      slack: false,
      teams: true,
    });
    const msgs = useAssistantStore.getState().messages;
    expect(msgs[msgs.length - 1]?.text).toBe("disk is full");
  });

  describe("the pending queue", () => {
    it("sends straight through without touching the queue while idle", async () => {
      await useAssistantStore.getState().send("hello");

      expect(assistantSend).toHaveBeenCalledWith("hello", []);
      expect(useAssistantStore.getState().pending).toEqual([]);
      expect(useAssistantStore.getState().messages.map((m) => m.role)).toEqual([
        "user",
      ]);
    });

    it("does not show an optimistic bubble for a message the backend queued", async () => {
      vi.mocked(assistantSend).mockResolvedValueOnce(false);

      const sent = await useAssistantStore.getState().send("queue me");

      expect(sent).toBe(true);
      expect(assistantSend).toHaveBeenCalledWith("queue me", []);
      // No local bubble: the backend's `queueChanged` event is what shows it.
      expect(useAssistantStore.getState().messages).toEqual([]);
    });

    // The bug this guards: an external-sources poll or a nudge is a turn the
    // panel is never told about, so `busy` reads false while the session is
    // in fact busy. Deciding on that instead of on the backend's answer put
    // the message on screen twice — as a sent bubble and as a waiting row.
    it("trusts the backend over its own busy flag when a turn it cannot see is running", async () => {
      vi.mocked(assistantSend).mockResolvedValueOnce(false);
      expect(useAssistantStore.getState().busy).toBe(false);

      await useAssistantStore.getState().send("start the release");
      useAssistantStore.getState().apply({
        type: "queueChanged",
        items: [
          {
            id: "p-1",
            kind: "user",
            text: "start the release",
            attachments: [],
          },
        ],
      });

      expect(useAssistantStore.getState().messages).toEqual([]);
      expect(useAssistantStore.getState().pending.map((p) => p.text)).toEqual([
        "start the release",
      ]);
    });

    it("mirrors a queueChanged snapshot exactly", () => {
      useAssistantStore.getState().apply({
        type: "queueChanged",
        items: [{ id: "p-1", kind: "user", text: "waiting", attachments: [] }],
      });

      expect(useAssistantStore.getState().pending).toEqual([
        { id: "p-1", kind: "user", text: "waiting", attachments: [] },
      ]);
    });

    it("shows a dropped/undelivered queue notice as a plain notice", () => {
      useAssistantStore.getState().apply({
        type: "queueNotice",
        text: "[Podium] The waiting-message queue is full (20); the oldest waiting message was dropped.",
      });

      const [row] = useAssistantStore.getState().messages;
      expect(row.role).toBe("notice");
      expect(row.text).toContain("dropped");
    });

    it("promotes a delivered waiting message into the log and re-busies the panel", () => {
      useAssistantStore.setState({
        busy: false,
        pending: [
          { id: "p-1", kind: "user", text: "its turn came", attachments: [] },
        ],
      });

      useAssistantStore.getState().apply({
        type: "queueItemDelivered",
        text: "its turn came",
      });

      expect(useAssistantStore.getState().busy).toBe(true);
      expect(
        useAssistantStore.getState().messages.map((m) => [m.role, m.text]),
      ).toEqual([["user", "its turn came"]]);
    });

    it("edits a waiting message optimistically and rolls back on failure", async () => {
      useAssistantStore.setState({
        pending: [
          { id: "p-1", kind: "user", text: "first draft", attachments: [] },
        ],
      });

      await useAssistantStore.getState().editPending("p-1", "final draft");
      expect(assistantQueueEdit).toHaveBeenCalledWith("p-1", "final draft");
      expect(useAssistantStore.getState().pending[0].text).toBe("final draft");

      vi.mocked(assistantQueueEdit).mockRejectedValueOnce({
        kind: "queueItemNotFound",
        message: "already delivered",
      });
      await useAssistantStore.getState().editPending("p-1", "too late");
      // Rolled back to what the backend last confirmed.
      expect(useAssistantStore.getState().pending[0].text).toBe("final draft");
      const msgs = useAssistantStore.getState().messages;
      expect(msgs[msgs.length - 1]?.text).toBe("already delivered");
    });

    it("ignores a blank edit rather than sending it", async () => {
      useAssistantStore.setState({
        pending: [
          { id: "p-1", kind: "user", text: "first draft", attachments: [] },
        ],
      });

      await useAssistantStore.getState().editPending("p-1", "   ");

      expect(assistantQueueEdit).not.toHaveBeenCalled();
      expect(useAssistantStore.getState().pending[0].text).toBe("first draft");
    });

    it("removes a waiting message optimistically and rolls back on failure", async () => {
      useAssistantStore.setState({
        pending: [
          { id: "p-1", kind: "user", text: "withdraw me", attachments: [] },
        ],
      });

      await useAssistantStore.getState().removePending("p-1");
      expect(assistantQueueRemove).toHaveBeenCalledWith("p-1");
      expect(useAssistantStore.getState().pending).toEqual([]);

      useAssistantStore.setState({
        pending: [
          { id: "p-2", kind: "user", text: "keep me", attachments: [] },
        ],
      });
      vi.mocked(assistantQueueRemove).mockRejectedValueOnce({
        kind: "queueItemNotFound",
        message: "already delivered",
      });
      await useAssistantStore.getState().removePending("p-2");
      expect(useAssistantStore.getState().pending).toEqual([
        { id: "p-2", kind: "user", text: "keep me", attachments: [] },
      ]);
    });

    it("interrupts the running turn", async () => {
      await useAssistantStore.getState().interrupt();
      expect(assistantInterrupt).toHaveBeenCalled();
    });

    it("reports a failed interrupt in the log", async () => {
      vi.mocked(assistantInterrupt).mockRejectedValueOnce({
        kind: "io",
        message: "could not kill the session",
      });
      await useAssistantStore.getState().interrupt();
      const msgs = useAssistantStore.getState().messages;
      expect(msgs[msgs.length - 1]?.text).toBe("could not kill the session");
    });

    it("drops the local queue on a fresh conversation", async () => {
      useAssistantStore.setState({
        pending: [{ id: "p-1", kind: "user", text: "stale", attachments: [] }],
      });

      await useAssistantStore.getState().newConversation();

      expect(useAssistantStore.getState().pending).toEqual([]);
    });
  });

  describe("notifying the user", () => {
    const { apply } = useAssistantStore.getState();

    it("notifies once at turn end, using the last streamed block as the body", () => {
      apply({ type: "message", text: "partial…" });
      apply({ type: "message", text: "Two to-dos are open." });
      apply({ type: "turnEnd", error: null, notify: true });

      expect(notifyAssistant).toHaveBeenCalledOnce();
      expect(notifyAssistant).toHaveBeenCalledWith(
        "Assistant replied",
        "Two to-dos are open.",
      );
    });

    it("stays silent when the window is focused and the pane is open", () => {
      vi.spyOn(document, "hasFocus").mockReturnValue(true);
      useLayoutStore.setState({ assistantOpen: true });

      apply({ type: "message", text: "Two to-dos are open." });
      apply({ type: "turnEnd", error: null, notify: true });

      expect(notifyAssistant).not.toHaveBeenCalled();
    });

    it("notifies when the pane is open but the window is unfocused", () => {
      vi.spyOn(document, "hasFocus").mockReturnValue(false);
      useLayoutStore.setState({ assistantOpen: true });

      apply({ type: "message", text: "reply" });
      apply({ type: "turnEnd", error: null, notify: true });

      expect(notifyAssistant).toHaveBeenCalledOnce();
    });

    it("notifies when the window is focused but the pane is closed", () => {
      vi.spyOn(document, "hasFocus").mockReturnValue(true);
      useLayoutStore.setState({ assistantOpen: false });

      apply({ type: "message", text: "reply" });
      apply({ type: "turnEnd", error: null, notify: true });

      expect(notifyAssistant).toHaveBeenCalledOnce();
    });

    it("does not notify when the turn produced nothing", () => {
      apply({ type: "turnEnd", error: null, notify: true });

      expect(notifyAssistant).not.toHaveBeenCalled();
    });

    it("stays silent for an unprompted turn the backend did not flag", () => {
      // A nudge the assistant answered with plain news — the panel shows it,
      // the desktop does not hear about it.
      apply({ type: "message", text: "The agent finished its to-do." });
      apply({ type: "turnEnd", error: null, notify: false });

      expect(notifyAssistant).not.toHaveBeenCalled();
    });

    it("does not notify when the turn failed", () => {
      apply({ type: "message", text: "reply" });
      apply({ type: "turnEnd", error: "error_max_turns", notify: true });

      expect(notifyAssistant).not.toHaveBeenCalled();
    });

    it("does not notify for a proactive nudge itself", () => {
      apply({ type: "notice", text: "[Podium] an agent finished" });

      expect(notifyAssistant).not.toHaveBeenCalled();
    });

    it("notifies when a proposal appears, needing confirm/reject", () => {
      apply({
        type: "proposal",
        id: "p-1",
        tool: "set_todo_archived",
        summary: '{"todoId":"t-1"}',
      });

      expect(notifyAssistant).toHaveBeenCalledOnce();
      expect(notifyAssistant).toHaveBeenCalledWith(
        "Assistant has a question",
        '{"todoId":"t-1"}',
      );
    });

    it("does not re-notify for a resolved proposal", () => {
      apply({
        type: "proposal",
        id: "p-1",
        tool: "set_todo_archived",
        summary: "",
      });
      notifyAssistant.mockClear();

      apply({
        type: "proposalResolved",
        id: "p-1",
        approved: true,
        error: null,
      });

      expect(notifyAssistant).not.toHaveBeenCalled();
    });

    it("resets the reply tracked per turn when a queued message is delivered", () => {
      apply({ type: "message", text: "first turn's reply" });
      apply({ type: "queueItemDelivered", text: "next queued message" });
      apply({ type: "turnEnd", error: null, notify: true });

      // The stale reply from before the queued turn started must not leak
      // into a notification for a turn that produced nothing new.
      expect(notifyAssistant).not.toHaveBeenCalled();
    });
  });
});
