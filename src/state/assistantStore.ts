/**
 * The personal assistant's chat log.
 *
 * In-memory only for now (phase 3 adds a persisted transcript). Turns are
 * appended locally when the user sends, and from the `assistant:event` stream
 * as the backend parses the headless session's output.
 */

import { create } from "zustand";

import { assistantSend, toIpcError } from "../ipc/commands";
import type { AssistantEvent } from "../ipc/types";

export type ChatRole = "user" | "assistant" | "tool" | "notice";

export interface ChatMessage {
  id: number;
  role: ChatRole;
  text: string;
}

interface AssistantState {
  messages: ChatMessage[];
  /** True from sending a turn until the assistant's reply ends. */
  busy: boolean;
  /** Send a turn; returns whether it was accepted (the input clears then). */
  send: (text: string) => Promise<boolean>;
  /** Fold one backend stream event into the log. */
  apply: (event: AssistantEvent) => void;
}

let seq = 0;

function message(role: ChatRole, text: string): ChatMessage {
  seq += 1;
  return { id: seq, role, text };
}

export const useAssistantStore = create<AssistantState>((set) => ({
  messages: [],
  busy: false,
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
