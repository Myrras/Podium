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
  assistantResponsesList: vi.fn(() => Promise.resolve([])),
  assistantResponseDismiss: vi.fn(() => Promise.resolve()),
  assistantResponseReply: vi.fn(() => Promise.resolve(true)),
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
  assistantResponseDismiss,
  assistantResponseReply,
  assistantResponsesList,
  assistantSend,
} from "../ipc/commands";
import type { ResponseItem } from "../ipc/types";

const notifyAssistant = vi.fn();
vi.mock("../lib/notify", () => ({
  notifyAssistant: (title: string, body: string) =>
    notifyAssistant(title, body),
}));

// Which agent the user is looking at is the activity store's business; the
// assistant store only asks.
const viewed = vi.fn<() => string | null>(() => null);
vi.mock("./agentActivityStore", () => ({
  viewedAgentId: () => viewed(),
}));

import { useLayoutStore } from "./layoutStore";
import { resetSeenResponses, useAssistantStore } from "./assistantStore";

function responseItem(overrides: Partial<ResponseItem> = {}): ResponseItem {
  return {
    id: "r-1",
    kind: "needsYou",
    text: "CSV or XLSX for the export?",
    project: "Webshop",
    agent: "Invoice export",
    agentColor: "#30a46c",
    todo: "Export invoices",
    source: null,
    processId: "proc-1",
    todoId: "todo-1",
    createdAt: "2026-01-01T10:00:00Z",
    label: "Invoice export · Webshop · Export invoices",
    ...overrides,
  };
}

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
      responses: [],
    });
  });

  describe("responses", () => {
    beforeEach(() => {
      vi.mocked(assistantResponsesList).mockReset();
      vi.mocked(assistantResponsesList).mockResolvedValue([]);
      vi.mocked(assistantResponseDismiss).mockReset();
      vi.mocked(assistantResponseDismiss).mockResolvedValue(undefined);
      vi.mocked(assistantResponseReply).mockReset();
      vi.mocked(assistantResponseReply).mockResolvedValue(true);
    });

    it("loads the list and mirrors responsesChanged snapshots", async () => {
      vi.mocked(assistantResponsesList).mockResolvedValueOnce([responseItem()]);
      await useAssistantStore.getState().loadResponses();
      expect(useAssistantStore.getState().responses).toHaveLength(1);

      useAssistantStore
        .getState()
        .apply({ type: "responsesChanged", items: [] });
      expect(useAssistantStore.getState().responses).toEqual([]);
    });

    it("drops a fetched list when a snapshot landed while it was in flight", async () => {
      let resolve!: (items: ResponseItem[]) => void;
      vi.mocked(assistantResponsesList).mockReturnValueOnce(
        new Promise((r) => (resolve = r)),
      );
      const loading = useAssistantStore.getState().loadResponses();
      const newer = responseItem({ id: "r-2", text: "newer" });
      useAssistantStore
        .getState()
        .apply({ type: "responsesChanged", items: [newer] });
      resolve([responseItem()]);
      await loading;
      expect(useAssistantStore.getState().responses).toEqual([newer]);
    });

    it("dismisses optimistically and reloads the backend's list on failure", async () => {
      useAssistantStore.setState({ responses: [responseItem()] });

      await useAssistantStore.getState().dismissResponse("r-1");
      expect(assistantResponseDismiss).toHaveBeenCalledWith("r-1");
      expect(useAssistantStore.getState().responses).toEqual([]);

      // Superseded meanwhile: the backend now holds a newer item, and the
      // failure must not bring the stale one back over it.
      const newer = responseItem({ id: "r-2", text: "newer" });
      useAssistantStore.setState({ responses: [responseItem()] });
      vi.mocked(assistantResponsesList).mockResolvedValueOnce([newer]);
      vi.mocked(assistantResponseDismiss).mockRejectedValueOnce({
        kind: "responseNotFound",
        message: "no such item",
      });
      await useAssistantStore.getState().dismissResponse("r-1");
      expect(useAssistantStore.getState().responses).toEqual([newer]);
      expect(useAssistantStore.getState().messages.slice(-1)[0]).toMatchObject({
        role: "notice",
        text: "no such item",
      });
    });

    it("adds a reply row when the reply is delivered", async () => {
      useAssistantStore.setState({ responses: [responseItem()] });

      const ok = await useAssistantStore
        .getState()
        .replyResponse("r-1", "  XLSX, VAT split per rate ");

      expect(ok).toBe(true);
      expect(assistantResponseReply).toHaveBeenCalledWith(
        "r-1",
        "XLSX, VAT split per rate",
      );
      expect(useAssistantStore.getState().busy).toBe(true);
      expect(
        useAssistantStore.getState().messages.map((m) => [m.role, m.text]),
      ).toEqual([
        [
          "reply",
          "Invoice export · Webshop · Export invoices\nXLSX, VAT split per rate",
        ],
      ]);
    });

    it("adds no row for a queued reply, and none for a blank one", async () => {
      useAssistantStore.setState({ responses: [responseItem()] });
      vi.mocked(assistantResponseReply).mockResolvedValueOnce(false);

      expect(
        await useAssistantStore.getState().replyResponse("r-1", "later"),
      ).toBe(true);
      expect(await useAssistantStore.getState().replyResponse("r-1", " ")).toBe(
        false,
      );
      expect(assistantResponseReply).toHaveBeenCalledOnce();
      expect(useAssistantStore.getState().messages).toEqual([]);
    });

    it("reports a failed reply as a notice", async () => {
      vi.mocked(assistantResponseReply).mockRejectedValueOnce({
        kind: "responseNotFound",
        message: "no such item",
      });

      expect(
        await useAssistantStore.getState().replyResponse("r-1", "hello"),
      ).toBe(false);
      expect(useAssistantStore.getState().messages.slice(-1)[0]?.role).toBe(
        "notice",
      );
    });

    it("promotes a delivered waiting reply into a reply row", () => {
      useAssistantStore.getState().apply({
        type: "queueItemDelivered",
        text: "XLSX",
        replyTo: "Invoice export · Webshop",
      });

      expect(
        useAssistantStore.getState().messages.map((m) => [m.role, m.text]),
      ).toEqual([["reply", "Invoice export · Webshop\nXLSX"]]);
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

    useAssistantStore.getState().apply({ type: "turnEnd", error: null });
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

    apply({ type: "turnEnd", error: null });
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
            replyTo: null,
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
        items: [
          {
            id: "p-1",
            kind: "user",
            text: "waiting",
            attachments: [],
            replyTo: null,
          },
        ],
      });

      expect(useAssistantStore.getState().pending).toEqual([
        {
          id: "p-1",
          kind: "user",
          text: "waiting",
          attachments: [],
          replyTo: null,
        },
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
          {
            id: "p-1",
            kind: "user",
            text: "its turn came",
            attachments: [],
            replyTo: null,
          },
        ],
      });

      useAssistantStore.getState().apply({
        type: "queueItemDelivered",
        text: "its turn came",
        replyTo: null,
      });

      expect(useAssistantStore.getState().busy).toBe(true);
      expect(
        useAssistantStore.getState().messages.map((m) => [m.role, m.text]),
      ).toEqual([["user", "its turn came"]]);
    });

    it("edits a waiting message optimistically and rolls back on failure", async () => {
      useAssistantStore.setState({
        pending: [
          {
            id: "p-1",
            kind: "user",
            text: "first draft",
            attachments: [],
            replyTo: null,
          },
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
          {
            id: "p-1",
            kind: "user",
            text: "first draft",
            attachments: [],
            replyTo: null,
          },
        ],
      });

      await useAssistantStore.getState().editPending("p-1", "   ");

      expect(assistantQueueEdit).not.toHaveBeenCalled();
      expect(useAssistantStore.getState().pending[0].text).toBe("first draft");
    });

    it("removes a waiting message optimistically and rolls back on failure", async () => {
      useAssistantStore.setState({
        pending: [
          {
            id: "p-1",
            kind: "user",
            text: "withdraw me",
            attachments: [],
            replyTo: null,
          },
        ],
      });

      await useAssistantStore.getState().removePending("p-1");
      expect(assistantQueueRemove).toHaveBeenCalledWith("p-1");
      expect(useAssistantStore.getState().pending).toEqual([]);

      useAssistantStore.setState({
        pending: [
          {
            id: "p-2",
            kind: "user",
            text: "keep me",
            attachments: [],
            replyTo: null,
          },
        ],
      });
      vi.mocked(assistantQueueRemove).mockRejectedValueOnce({
        kind: "queueItemNotFound",
        message: "already delivered",
      });
      await useAssistantStore.getState().removePending("p-2");
      expect(useAssistantStore.getState().pending).toEqual([
        {
          id: "p-2",
          kind: "user",
          text: "keep me",
          attachments: [],
          replyTo: null,
        },
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
        pending: [
          {
            id: "p-1",
            kind: "user",
            text: "stale",
            attachments: [],
            replyTo: null,
          },
        ],
      });

      await useAssistantStore.getState().newConversation();

      expect(useAssistantStore.getState().pending).toEqual([]);
    });
  });

  describe("notifying the user", () => {
    const { apply } = useAssistantStore.getState();
    const snapshot = (...items: ResponseItem[]) =>
      apply({ type: "responsesChanged", items });

    // The first load is the baseline: what it holds is not news.
    beforeEach(async () => {
      resetSeenResponses();
      viewed.mockReturnValue(null);
      vi.mocked(assistantResponsesList).mockReset();
      vi.mocked(assistantResponsesList).mockResolvedValue([]);
      await useAssistantStore.getState().loadResponses();
      notifyAssistant.mockClear();
    });

    it("does not ping for a reply, however a turn ends", () => {
      apply({ type: "message", text: "Two to-dos are open." });
      apply({ type: "turnEnd", error: null });
      apply({ type: "message", text: "reply" });
      apply({ type: "turnEnd", error: "error_max_turns" });

      expect(notifyAssistant).not.toHaveBeenCalled();
    });

    it("does not ping for a proposal card or a nudge itself", () => {
      apply({
        type: "proposal",
        id: "p-1",
        tool: "set_todo_archived",
        summary: '{"todoId":"t-1"}',
      });
      apply({ type: "notice", text: "[Podium] an agent finished" });
      apply({
        type: "proposalResolved",
        id: "p-1",
        approved: true,
        error: null,
      });

      expect(notifyAssistant).not.toHaveBeenCalled();
    });

    it("pings for a new needs-you item with its text", () => {
      snapshot(responseItem());

      expect(notifyAssistant).toHaveBeenCalledOnce();
      expect(notifyAssistant).toHaveBeenCalledWith(
        "Needs you",
        "CSV or XLSX for the export?",
      );
    });

    it("titles an fyi-only snapshot FYI", () => {
      snapshot(responseItem({ kind: "fyi", text: "PR is up" }));

      expect(notifyAssistant).toHaveBeenCalledWith("FYI", "PR is up");
    });

    it("pings for a proposal's item like any other needs-you", () => {
      snapshot(
        responseItem({
          id: "r-p",
          text: "Confirm or reject: stop_process {}",
          processId: null,
          todoId: null,
          source: "proposal",
        }),
      );

      expect(notifyAssistant).toHaveBeenCalledWith(
        "Needs you",
        "Confirm or reject: stop_process {}",
      );
    });

    it("never pings for what the first load already held", async () => {
      resetSeenResponses();
      vi.mocked(assistantResponsesList).mockResolvedValueOnce([responseItem()]);
      await useAssistantStore.getState().loadResponses();

      snapshot(responseItem());
      // A later snapshot that still carries it is no news either.
      snapshot(responseItem(), responseItem({ id: "r-2", text: "new" }));

      expect(notifyAssistant).toHaveBeenCalledOnce();
      expect(notifyAssistant).toHaveBeenCalledWith("Needs you", "new");
    });

    it("pings nothing before the list was loaded once", () => {
      resetSeenResponses();

      snapshot(responseItem());

      expect(notifyAssistant).not.toHaveBeenCalled();
    });

    it("does not ping again for a restored item (same id)", () => {
      snapshot(responseItem());
      snapshot(); // the user replied: it left the list
      snapshot(responseItem()); // the reply never went out: it is back

      expect(notifyAssistant).toHaveBeenCalledOnce();
    });

    it("pings for an item that replaced an open one (new id)", () => {
      snapshot(responseItem());
      snapshot(responseItem({ id: "r-2", text: "XLSX or PDF?" }));

      expect(notifyAssistant).toHaveBeenCalledTimes(2);
      expect(notifyAssistant).toHaveBeenLastCalledWith(
        "Needs you",
        "XLSX or PDF?",
      );
    });

    it("makes one notification of several new items", () => {
      // Newest first, like the backend's list.
      snapshot(
        responseItem({ id: "r-3", kind: "fyi", text: "Review posted" }),
        responseItem({ id: "r-2", kind: "needsYou", text: "Pick a variant" }),
        responseItem({ id: "r-1", kind: "fyi", text: "PR is up" }),
      );

      expect(notifyAssistant).toHaveBeenCalledOnce();
      expect(notifyAssistant).toHaveBeenCalledWith(
        "Needs you",
        "Review posted (+2 more)",
      );
    });

    it("counts only the new items, not the ones already on the list", () => {
      snapshot(responseItem({ id: "r-1", kind: "fyi", text: "PR is up" }));
      notifyAssistant.mockClear();

      snapshot(
        responseItem({ id: "r-2", kind: "fyi", text: "Review posted" }),
        responseItem({ id: "r-1", kind: "fyi", text: "PR is up" }),
      );

      expect(notifyAssistant).toHaveBeenCalledWith("FYI", "Review posted");
    });

    it("stays silent when the window is focused and the pane is open", () => {
      vi.spyOn(document, "hasFocus").mockReturnValue(true);
      useLayoutStore.setState({ assistantOpen: true });

      snapshot(responseItem());
      expect(notifyAssistant).not.toHaveBeenCalled();

      // Seen is seen: leaving the pane later does not ping for it.
      useLayoutStore.setState({ assistantOpen: false });
      snapshot(responseItem());
      expect(notifyAssistant).not.toHaveBeenCalled();
    });

    it("pings when the pane is open but the window is unfocused", () => {
      vi.spyOn(document, "hasFocus").mockReturnValue(false);
      useLayoutStore.setState({ assistantOpen: true });

      snapshot(responseItem());

      expect(notifyAssistant).toHaveBeenCalledOnce();
    });

    it("pings when the window is focused but the pane is closed", () => {
      vi.spyOn(document, "hasFocus").mockReturnValue(true);
      useLayoutStore.setState({ assistantOpen: false });

      snapshot(responseItem());

      expect(notifyAssistant).toHaveBeenCalledOnce();
    });

    it("skips an item about the agent the user is viewing", () => {
      viewed.mockReturnValue("proc-1");

      snapshot(responseItem({ processId: "proc-1" }));

      expect(notifyAssistant).not.toHaveBeenCalled();
    });

    it("pings for the other items when one is about the viewed agent", () => {
      viewed.mockReturnValue("proc-1");

      snapshot(
        responseItem({ id: "r-2", kind: "fyi", text: "Other", processId: "x" }),
        responseItem({ id: "r-1", processId: "proc-1" }),
      );

      expect(notifyAssistant).toHaveBeenCalledOnce();
      expect(notifyAssistant).toHaveBeenCalledWith("FYI", "Other");
    });
  });
});
