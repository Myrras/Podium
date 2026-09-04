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
  assistantInterrupt,
  assistantMemoryGet,
  assistantMemorySet,
  assistantNewConversation,
  assistantProactivityGet,
  assistantProactivitySet,
  assistantQueueEdit,
  assistantQueueRemove,
  assistantResolveProposal,
  assistantSend,
  assistantTranscript,
  toIpcError,
} from "../ipc/commands";
import type { AssistantEvent, AutonomyMode, QueuedMessage } from "../ipc/types";

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
  | "nudge";

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
  /** True from sending a turn until the assistant's reply ends. */
  busy: boolean;
  /** The tool currently running, or `null` between tool calls (still busy). */
  activity: string | null;
  /** How much the assistant may do unasked; mirrors the persisted setting. */
  autonomy: AutonomyMode;
  /** Whether Podium nudges the assistant unprompted; mirrors the setting. */
  proactive: boolean;
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
  /** Replay the persisted transcript into the log (once, on mount). */
  hydrate: () => Promise<void>;
  /** Drop the conversation and start over (memory is kept). */
  newConversation: () => Promise<void>;
  /** Load the stored memory (when the user opens the memory view). */
  loadMemory: () => Promise<void>;
  /** Replace the memory; `""` clears it. */
  saveMemory: (content: string) => Promise<void>;
  /** Send a turn; returns whether it was accepted (the input clears then). */
  send: (text: string) => Promise<boolean>;
  /** Load the stored autonomy mode (on mount). */
  loadAutonomy: () => Promise<void>;
  /** Change the autonomy mode. */
  setAutonomy: (mode: AutonomyMode) => Promise<void>;
  /** Load the stored proactivity setting (Settings → Assistant). */
  loadProactivity: () => Promise<void>;
  /** Turn proactive nudges on or off. */
  setProactivity: (enabled: boolean) => Promise<void>;
  /** Confirm or reject a proposed action. */
  resolveProposal: (id: string, approved: boolean) => Promise<void>;
  /** Change a waiting user message's text before it is delivered. */
  editPending: (id: string, text: string) => Promise<void>;
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
  memory: null,
  hydrated: false,
  pending: [],
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
  send: async (text) => {
    const trimmed = text.trim();
    if (!trimmed) return false;
    // Busy already means a turn is running: the backend will queue this one
    // rather than deliver it, and the `queueChanged` event that follows adds
    // its own waiting row — an optimistic "user" bubble here would show it
    // twice, once as if sent and once as waiting.
    const idle = !get().busy;
    if (idle) {
      set((s) => ({
        messages: [...s.messages, message("user", trimmed)],
        busy: true,
      }));
    }
    try {
      await assistantSend(trimmed);
      return true;
    } catch (e) {
      const err = toIpcError(e);
      // Failures belong in the chat, not only in a toast: the panel is the
      // one place the user follows the conversation.
      set((s) => ({
        messages: [...s.messages, message("notice", err.message)],
        ...(idle ? { busy: false } : {}),
      }));
      return false;
    }
  },
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
  resolveProposal: async (id, approved) => {
    // The decision goes to the assistant as a new turn, so the panel is busy
    // again either way.
    set((s) => ({
      messages: patchProposal(s.messages, id, {
        status: approved ? "approved" : "rejected",
      }),
      busy: true,
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
  apply: (event) =>
    set((s) => {
      switch (event.type) {
        case "message":
          return {
            messages: [...s.messages, message("assistant", event.text)],
          };
        case "tool":
          return {
            activity: event.name,
            messages: [...s.messages, message("tool", event.name)],
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
          return {
            busy: false,
            activity: null,
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
            messages: [...s.messages, message("user", event.text)],
          };
      }
    }),
}));
