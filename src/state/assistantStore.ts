/**
 * The personal assistant's chat log.
 *
 * In-memory only for now (phase 3 adds a persisted transcript). Turns are
 * appended locally when the user sends, and from the `assistant:event` stream
 * as the backend parses the headless session's output. Actions the assistant
 * ran show as cards in the same log, and actions the autonomy gate held back
 * show as proposal cards the user confirms or rejects here.
 */

import { create } from "zustand";

import {
  assistantAutonomyGet,
  assistantAutonomySet,
  assistantResolveProposal,
  assistantSend,
  toIpcError,
} from "../ipc/commands";
import type { AssistantEvent, AutonomyMode } from "../ipc/types";

export type ChatRole = "user" | "assistant" | "tool" | "notice" | "proposal";

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
  /** How much the assistant may do unasked; mirrors the persisted setting. */
  autonomy: AutonomyMode;
  /** Send a turn; returns whether it was accepted (the input clears then). */
  send: (text: string) => Promise<boolean>;
  /** Load the stored autonomy mode (on mount). */
  loadAutonomy: () => Promise<void>;
  /** Change the autonomy mode. */
  setAutonomy: (mode: AutonomyMode) => Promise<void>;
  /** Confirm or reject a proposed action. */
  resolveProposal: (id: string, approved: boolean) => Promise<void>;
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
  autonomy: "confirm-destructive",
  send: async (text) => {
    const trimmed = text.trim();
    if (!trimmed) return false;
    set((s) => ({
      messages: [...s.messages, message("user", trimmed)],
      busy: true,
    }));
    try {
      await assistantSend(trimmed);
      return true;
    } catch (e) {
      const err = toIpcError(e);
      // Failures belong in the chat, not only in a toast: the panel is the
      // one place the user follows the conversation.
      set((s) => ({
        messages: [...s.messages, message("notice", err.message)],
        busy: false,
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
  apply: (event) =>
    set((s) => {
      switch (event.type) {
        case "message":
          return {
            messages: [...s.messages, message("assistant", event.text)],
          };
        case "tool":
          return {
            messages: [...s.messages, message("tool", event.name)],
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
            messages: [
              ...s.messages,
              message(
                "notice",
                "The assistant session ended. Send another message to start a new one.",
              ),
            ],
          };
        // `started` carries Claude's session id — only phase 3 (resume) needs
        // it, and it is not chat content.
        case "started":
          return s;
      }
    }),
}));
