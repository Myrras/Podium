/**
 * The personal assistant's chat log.
 *
 * The backend persists the conversation, so the log is *hydrated* from the
 * stored transcript on startup and grows from there: turns are appended
 * locally when the user sends, and from the `assistant:event` stream as the
 * backend parses the headless session's output. Actions the assistant ran show
 * as cards in the same log, and actions the autonomy gate held back show as
 * proposal cards the user confirms or rejects here. Podium's proactive nudges
 * arrive as notice rows. Starting a new conversation clears both sides.
 */

import { create } from "zustand";

import {
  assistantAutonomyGet,
  assistantAutonomySet,
  assistantDayStart,
  assistantInterrupt,
  assistantMemoryGet,
  assistantMemorySet,
  assistantNewConversation,
  assistantPollingGet,
  assistantPollingSet,
  assistantProactivityGet,
  assistantProactivitySet,
  assistantQueueEdit,
  assistantQueueRemove,
  assistantQueueSendNow,
  assistantResolveProposal,
  assistantResponseDismiss,
  assistantResponseReply,
  assistantResponsesList,
  assistantSend,
  assistantTranscript,
  toIpcError,
} from "../ipc/commands";
import type {
  Attachment,
  AssistantEvent,
  AutonomyMode,
  PollingSettings,
  QueuedMessage,
  ResponseItem,
} from "../ipc/types";
import { notifyAssistant } from "../lib/notify";
import { useLayoutStore } from "./layoutStore";

/**
 * Is the user already looking at the assistant's reply? True only when the
 * app window is focused and the chat pane is open — the same "don't ping
 * the window you're watching" rule `agentActivityStore` applies to agents.
 */
function isAssistantVisible(): boolean {
  return document.hasFocus() && useLayoutStore.getState().assistantOpen;
}

export type ChatRole =
  | "user"
  | "assistant"
  | "tool"
  | "notice"
  | "proposal"
  /**
   * Podium spoke to the *assistant*, not the user — an internal nudge (an
   * agent finished, a to-do is lingering, …) carrying instruction text meant
   * for the assistant to act on. Kept out of `"notice"` so the log never
   * renders it as a message written to the user; it gets a collapsed, muted
   * marker instead. The backend persists nudges under their own
   * `TranscriptRole::Nudge`, distinct from a real notice (e.g. a tool
   * failure), so a replayed nudge renders the same way after a restart.
   */
  | "nudge"
  /**
   * The user's reply to a Responses item. `text` is `"<label>\n<reply>"` —
   * the first line names the item it answers, the same shape the backend
   * stores under `TranscriptRole::Reply`, so a replayed reply renders alike.
   */
  | "reply";

/**
 * Bumped on every `responsesChanged` snapshot, so a `loadResponses` fetch
 * that started before one can tell its answer is stale.
 */
let responsesSnapshotGen = 0;

/** A gated action awaiting (or carrying) the user's decision. */
export interface ChatProposal {
  id: string;
  /** MCP tool the assistant wants to run, e.g. `set_todo_archived`. */
  tool: string;
  /** Its arguments, so the user can see what they are approving. */
  summary: string;
  status: "pending" | "approved" | "rejected" | "failed";
  /** Why the confirmed action failed, if it did. */
  error?: string;
}

export interface ChatMessage {
  id: number;
  role: ChatRole;
  text: string;
  /** Set on `proposal` rows — the card's state. */
  proposal?: ChatProposal;
}

interface AssistantState {
  messages: ChatMessage[];
  /**
   * True from the moment a turn Podium told us about started until its reply
   * ends. Only turns the panel can see count: an external-sources poll or a
   * nudge runs without publishing anything, so this can be `false` while the
   * session is in fact busy. Never use it to predict whether a message will
   * be delivered or queued — the backend answers that (see `send`).
   */
  busy: boolean;
  /** The tool currently running, or `null` between tool calls (still busy). */
  activity: string | null;
  /** How much the assistant may do unasked; mirrors the persisted setting. */
  autonomy: AutonomyMode;
  /** Whether Podium nudges the assistant unprompted; mirrors the setting. */
  proactive: boolean;
  /**
   * The proactive external-source (Slack/Teams) polling sweep; mirrors the
   * persisted setting. Seeded with the backend's own defaults so the tab
   * shows a sane value before the load round-trip completes.
   */
  polling: PollingSettings;
  /** What the assistant remembers across conversations; `null` until loaded. */
  memory: string | null;
  /** Whether the stored transcript was already replayed this app run. */
  hydrated: boolean;
  /**
   * Messages waiting for the current turn to end (or for the assistant's
   * session to exist at all), oldest first — mirrors the backend's queue
   * exactly via `queueChanged` events; not persisted, so it starts empty on
   * every launch.
   */
  pending: QueuedMessage[];
  /**
   * Text of the latest assistant reply streamed this turn, or `null` before
   * one arrives. Used to notify once at `turnEnd` — using the *last*
   * streamed block, not one ping per block — and reset whenever a new turn
   * starts.
   */
  turnReplyText: string | null;
  /**
   * The Responses list, newest first — mirrors the backend's persisted list
   * via `responsesChanged` snapshots.
   */
  responses: ResponseItem[];
  /** Load the Responses list (on mount). */
  loadResponses: () => Promise<void>;
  /** Tick a Responses item off. */
  dismissResponse: (id: string) => Promise<void>;
  /** Reply to a Responses item; returns whether it was accepted. */
  replyResponse: (id: string, text: string) => Promise<boolean>;
  /** Replay the persisted transcript into the log (once, on mount). */
  hydrate: () => Promise<void>;
  /** Drop the conversation and start over (memory is kept). */
  newConversation: () => Promise<void>;
  /** Load the stored memory (when the user opens the memory view). */
  loadMemory: () => Promise<void>;
  /** Replace the memory; `""` clears it. */
  saveMemory: (content: string) => Promise<void>;
  /** Send a turn; returns whether it was accepted (the input clears then). */
  send: (text: string, attachments?: Attachment[]) => Promise<boolean>;
  /** Trigger the day-start summary as a normal user turn. */
  dayStart: () => Promise<void>;
  /** Push a local notice (e.g. a client-side attachment-limit rejection). */
  pushNotice: (text: string) => void;
  /** Load the stored autonomy mode (on mount). */
  loadAutonomy: () => Promise<void>;
  /** Change the autonomy mode. */
  setAutonomy: (mode: AutonomyMode) => Promise<void>;
  /** Load the stored proactivity setting (Settings → Assistant). */
  loadProactivity: () => Promise<void>;
  /** Turn proactive nudges on or off. */
  setProactivity: (enabled: boolean) => Promise<void>;
  /** Load the stored polling settings (Settings → Assistant). */
  loadPolling: () => Promise<void>;
  /** Replace the polling settings (interval + per-source toggles) as one. */
  setPolling: (settings: PollingSettings) => Promise<void>;
  /** Confirm or reject a proposed action. */
  resolveProposal: (id: string, approved: boolean) => Promise<void>;
  /** Change a waiting user message's text before it is delivered. */
  editPending: (id: string, text: string) => Promise<void>;
  /** Push a waiting user message into the running turn now. */
  sendPendingNow: (id: string) => Promise<void>;
  /** Withdraw a waiting user message. */
  removePending: (id: string) => Promise<void>;
  /** Stop the current turn, if any, and hand the next waiting message over. */
  interrupt: () => Promise<void>;
  /** Fold one backend stream event into the log. */
  apply: (event: AssistantEvent) => void;
}

let seq = 0;

function message(
  role: ChatRole,
  text: string,
  proposal?: ChatProposal,
): ChatMessage {
  seq += 1;
  return { id: seq, role, text, proposal };
}

/** Replace one proposal card's state, leaving the rest of the log alone. */
function patchProposal(
  messages: ChatMessage[],
  id: string,
  patch: Partial<ChatProposal>,
): ChatMessage[] {
  return messages.map((m) =>
    m.proposal?.id === id ? { ...m, proposal: { ...m.proposal, ...patch } } : m,
  );
}

export const useAssistantStore = create<AssistantState>((set, get) => ({
  messages: [],
  busy: false,
  activity: null,
  autonomy: "confirm-destructive",
  proactive: true,
  polling: { intervalMinutes: 10, slack: true, teams: true },
  memory: null,
  hydrated: false,
  pending: [],
  turnReplyText: null,
  responses: [],
  loadResponses: async () => {
    const gen = responsesSnapshotGen;
    try {
      const responses = await assistantResponsesList();
      // A snapshot that landed while this was in flight is newer.
      if (gen === responsesSnapshotGen) set({ responses });
    } catch {
      // An empty list is a fine fallback; the next snapshot corrects it.
    }
  },
  dismissResponse: async (id) => {
    set((s) => ({ responses: s.responses.filter((r) => r.id !== id) }));
    try {
      await assistantResponseDismiss(id);
    } catch (e) {
      // Back to the backend's list rather than the one from before the
      // click: a snapshot may have landed meanwhile (the item superseded,
      // a new one filed), and restoring the old array would undo it.
      set((s) => ({
        messages: [...s.messages, message("notice", toIpcError(e).message)],
      }));
      await get().loadResponses();
    }
  },
  replyResponse: async (id, text) => {
    const trimmed = text.trim();
    if (!trimmed) return false;
    const item = get().responses.find((r) => r.id === id);
    const label = item?.label ?? "Responses item";
    try {
      // Same delivered-or-queued answer as `send`: a queued reply shows as a
      // waiting row instead. The item itself leaves via `responsesChanged`.
      if (await assistantResponseReply(id, trimmed)) {
        set((s) => ({
          messages: [...s.messages, message("reply", `${label}\n${trimmed}`)],
          busy: true,
          turnReplyText: null,
        }));
      }
      return true;
    } catch (e) {
      set((s) => ({
        messages: [...s.messages, message("notice", toIpcError(e).message)],
      }));
      return false;
    }
  },
  hydrate: async () => {
    // Once per app run: the panel remounts every time it is reopened, and the
    // log it already holds is the same conversation.
    if (get().hydrated) return;
    set({ hydrated: true });
    try {
      const stored = await assistantTranscript();
      // Prepended, not assigned: a turn may have streamed in while this was
      // in flight, and it belongs after the stored rows.
      set((s) => ({
        messages: [
          ...stored.map((e) => message(e.role, e.text)),
          ...s.messages,
        ],
      }));
    } catch {
      // An empty panel is a fine fallback — the conversation is still there.
    }
  },
  newConversation: async () => {
    try {
      await assistantNewConversation();
      // The backend may have recorded an "undelivered messages" notice as
      // the first row of the now-otherwise-empty transcript (the queue is
      // dropped too, quietly — no `queueChanged` event is sent for it), so
      // re-fetch rather than assuming the log is simply empty.
      const stored = await assistantTranscript();
      set({
        messages: stored.map((e) => message(e.role, e.text)),
        busy: false,
        activity: null,
        pending: [],
        turnReplyText: null,
      });
    } catch (e) {
      set((s) => ({
        messages: [...s.messages, message("notice", toIpcError(e).message)],
      }));
    }
  },
  loadMemory: async () => {
    try {
      set({ memory: await assistantMemoryGet() });
    } catch (e) {
      set((s) => ({
        memory: "",
        messages: [...s.messages, message("notice", toIpcError(e).message)],
      }));
    }
  },
  saveMemory: async (content) => {
    try {
      await assistantMemorySet(content);
      set({ memory: content });
    } catch (e) {
      set((s) => ({
        messages: [...s.messages, message("notice", toIpcError(e).message)],
      }));
    }
  },
  send: async (text, attachments = []) => {
    const trimmed = text.trim();
    if (!trimmed) return false;
    try {
      // The backend says whether it delivered or queued; only a delivered
      // message gets a chat bubble, because a queued one already shows as a
      // waiting row via `queueChanged`. `busy` is not usable as a stand-in
      // here: a turn Podium started for itself (the external-sources poll, a
      // nudge) publishes nothing the panel can see, so the panel reads as
      // idle while the session is busy — and the bubble it added on that
      // guess put the same message on screen twice, once as sent and once as
      // waiting.
      if (await assistantSend(trimmed, attachments)) {
        set((s) => ({
          messages: [...s.messages, message("user", trimmed)],
          busy: true,
          turnReplyText: null,
        }));
      }
      return true;
    } catch (e) {
      const err = toIpcError(e);
      // Failures belong in the chat, not only in a toast: the panel is the
      // one place the user follows the conversation.
      set((s) => ({
        messages: [...s.messages, message("notice", err.message)],
      }));
      return false;
    }
  },
  dayStart: async () => {
    try {
      // Echoed like `send`, and on the same delivered-or-queued answer: a
      // user turn publishes no event of its own, so without the bubble the
      // panel sits empty for the seconds the CLI takes to boot. The bubble is
      // the button's own wording; the transcript keeps the longer prompt the
      // backend actually sent. Queued instead, the waiting row covers it.
      if (await assistantDayStart()) {
        set((s) => ({
          messages: [...s.messages, message("user", "Start my day")],
          busy: true,
          turnReplyText: null,
        }));
      }
    } catch (e) {
      set((s) => ({
        messages: [...s.messages, message("notice", toIpcError(e).message)],
      }));
    }
  },
  pushNotice: (text) =>
    set((s) => ({ messages: [...s.messages, message("notice", text)] })),
  loadAutonomy: async () => {
    try {
      set({ autonomy: await assistantAutonomyGet() });
    } catch {
      // The panel still works on the default; the backend stays the truth.
    }
  },
  setAutonomy: async (mode) => {
    const previous = get().autonomy;
    set({ autonomy: mode });
    try {
      set({ autonomy: await assistantAutonomySet(mode) });
    } catch (e) {
      set((s) => ({
        autonomy: previous,
        messages: [...s.messages, message("notice", toIpcError(e).message)],
      }));
    }
  },
  loadProactivity: async () => {
    try {
      set({ proactive: await assistantProactivityGet() });
    } catch {
      // The panel still works on the default; the backend stays the truth.
    }
  },
  setProactivity: async (enabled) => {
    const previous = get().proactive;
    set({ proactive: enabled });
    try {
      set({ proactive: await assistantProactivitySet(enabled) });
    } catch (e) {
      set((s) => ({
        proactive: previous,
        messages: [...s.messages, message("notice", toIpcError(e).message)],
      }));
    }
  },
  loadPolling: async () => {
    try {
      set({ polling: await assistantPollingGet() });
    } catch {
      // The panel still works on the default; the backend stays the truth.
    }
  },
  setPolling: async (settings) => {
    const previous = get().polling;
    set({ polling: settings });
    try {
      set({
        polling: await assistantPollingSet(
          settings.intervalMinutes,
          settings.slack,
          settings.teams,
        ),
      });
    } catch (e) {
      set((s) => ({
        polling: previous,
        messages: [...s.messages, message("notice", toIpcError(e).message)],
      }));
    }
  },
  resolveProposal: async (id, approved) => {
    // The decision goes to the assistant as a new turn, so the panel is busy
    // again either way.
    set((s) => ({
      messages: patchProposal(s.messages, id, {
        status: approved ? "approved" : "rejected",
      }),
      busy: true,
      turnReplyText: null,
    }));
    try {
      await assistantResolveProposal(id, approved);
    } catch (e) {
      set((s) => ({
        messages: [
          ...patchProposal(s.messages, id, { status: "pending" }),
          message("notice", toIpcError(e).message),
        ],
        busy: false,
      }));
    }
  },
  editPending: async (id, text) => {
    const trimmed = text.trim();
    if (!trimmed) return;
    const previous = get().pending;
    // Optimistic: `queueChanged` echoes the change back anyway, but this
    // keeps the box feeling responsive while the round trip is in flight.
    set({
      pending: previous.map((p) => (p.id === id ? { ...p, text: trimmed } : p)),
    });
    try {
      await assistantQueueEdit(id, trimmed);
    } catch (e) {
      set((s) => ({
        pending: previous,
        messages: [...s.messages, message("notice", toIpcError(e).message)],
      }));
    }
  },
  sendPendingNow: async (id) => {
    const previous = get().pending;
    // Optimistic like the other queue actions: the row leaves the waiting
    // list at once, and `queueChanged`/`queueItemDelivered` confirm it.
    set({ pending: previous.filter((p) => p.id !== id) });
    try {
      await assistantQueueSendNow(id);
    } catch (e) {
      set((s) => ({
        pending: previous,
        messages: [...s.messages, message("notice", toIpcError(e).message)],
      }));
    }
  },
  removePending: async (id) => {
    const previous = get().pending;
    set({ pending: previous.filter((p) => p.id !== id) });
    try {
      await assistantQueueRemove(id);
    } catch (e) {
      set((s) => ({
        pending: previous,
        messages: [...s.messages, message("notice", toIpcError(e).message)],
      }));
    }
  },
  interrupt: async () => {
    try {
      await assistantInterrupt();
    } catch (e) {
      set((s) => ({
        messages: [...s.messages, message("notice", toIpcError(e).message)],
      }));
    }
  },
  apply: (event) => {
    if (event.type === "responsesChanged") responsesSnapshotGen++;
    set((s) => {
      switch (event.type) {
        case "message":
          return {
            turnReplyText: event.text,
            messages: [...s.messages, message("assistant", event.text)],
          };
        case "tool":
          return {
            activity: event.name,
            // `add_response` is bookkeeping: the Responses list shows its
            // result, so it gets no action row (the transcript skips it too).
            messages: /(^|__)add_response$/.test(event.name)
              ? s.messages
              : [...s.messages, message("tool", event.name)],
          };
        // Podium nudging the assistant, not the user: demoted so it never
        // reads as a message written to them (see the `"nudge"` ChatRole doc).
        case "notice":
          return {
            messages: [...s.messages, message("nudge", event.text)],
          };
        case "toolFailed":
          return {
            messages: [
              ...s.messages,
              message("notice", `That action failed: ${event.message}`),
            ],
          };
        case "proposal":
          // The assistant is waiting on the user, exactly like a stalled
          // agent — ping unless they're already watching the pane.
          if (!isAssistantVisible()) {
            notifyAssistant(
              "Assistant has a question",
              event.summary || event.tool,
            );
          }
          return {
            messages: [
              ...s.messages,
              message("proposal", "", {
                id: event.id,
                tool: event.tool,
                summary: event.summary,
                status: "pending",
              }),
            ],
          };
        case "proposalResolved":
          return {
            messages: patchProposal(s.messages, event.id, {
              status: event.error
                ? "failed"
                : event.approved
                  ? "approved"
                  : "rejected",
              error: event.error ?? undefined,
            }),
          };
        case "turnEnd":
          // One ping per turn, using the last reply streamed during it — not
          // one per streamed block — and only when nothing came back is
          // there nothing to tell the user about. `notify` is the backend's
          // call: an unprompted turn only reaches the desktop when the
          // assistant said the user has to act, so a run of "agent finished"
          // reports stays in the panel.
          if (
            !event.error &&
            s.turnReplyText &&
            event.notify &&
            !isAssistantVisible()
          ) {
            notifyAssistant("Assistant replied", s.turnReplyText);
          }
          return {
            busy: false,
            activity: null,
            turnReplyText: null,
            messages: event.error
              ? [
                  ...s.messages,
                  message("notice", `The turn failed: ${event.error}`),
                ]
              : s.messages,
          };
        case "exited":
          return {
            busy: false,
            activity: null,
            messages: [
              ...s.messages,
              message(
                "notice",
                "The assistant session ended. Send another message to start a new one.",
              ),
            ],
          };
        // `started` carries Claude's session id: the backend stores it to
        // resume the conversation later, and it is not chat content.
        case "started":
          return s;
        case "queueChanged":
          return { pending: event.items };
        case "queueNotice":
          return {
            messages: [...s.messages, message("notice", event.text)],
          };
        // A waiting message's turn came up — nothing else announces this,
        // so promote it into the ordinary log and show the panel as busy
        // again (mirrors the optimistic bubble a same-cycle `send()` shows
        // for a message delivered immediately).
        case "queueItemDelivered":
          return {
            busy: true,
            turnReplyText: null,
            messages: [
              ...s.messages,
              event.replyTo
                ? message("reply", `${event.replyTo}\n${event.text}`)
                : message("user", event.text),
            ],
          };
        case "responsesChanged":
          return { responses: event.items };
      }
    });
  },
}));
