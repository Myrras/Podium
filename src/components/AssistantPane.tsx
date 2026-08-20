/**
 * The personal assistant chat panel — a collapsible island on the right of
 * the app, workspace-wide (it is not tied to the selected project).
 *
 * Phase 1 is the conversation itself: the user's turns, the assistant's prose
 * (markdown), the tools it used, and failures. Confirming proposed actions,
 * persistence and proactive nudges come later.
 */

import { useEffect, useRef, useState } from "react";

import { useAssistantStore } from "../state/assistantStore";
import { useLayoutStore } from "../state/layoutStore";
import { Markdown } from "./Markdown";
import { AssistantIcon, CloseIcon, SendIcon } from "./icons";
import styles from "./AssistantPane.module.css";

export function AssistantPane() {
  const messages = useAssistantStore((s) => s.messages);
  const busy = useAssistantStore((s) => s.busy);
  const send = useAssistantStore((s) => s.send);
  const setAssistantOpen = useLayoutStore((s) => s.setAssistantOpen);
  const [draft, setDraft] = useState("");
  const logRef = useRef<HTMLDivElement>(null);

  // Follow the tail as turns stream in.
  useEffect(() => {
    const log = logRef.current;
    if (log) log.scrollTop = log.scrollHeight;
  }, [messages, busy]);

  async function submit() {
    const text = draft.trim();
    if (!text) return;
    // Clear optimistically: the store keeps the turn either way (a failure
    // lands in the log as a notice), so re-typing it would be noise.
    setDraft("");
    await send(text);
  }

  return (
    <aside className={styles.pane} aria-label="Personal assistant">
      <header className={styles.header}>
        <AssistantIcon className={styles.headerIcon} />
        <span className={styles.title}>Assistant</span>
        <button
          type="button"
          className={styles.closeBtn}
          aria-label="Hide assistant"
          title="Hide assistant"
          onClick={() => setAssistantOpen(false)}
        >
          <CloseIcon />
        </button>
      </header>

      <div className={styles.log} ref={logRef}>
        {messages.length === 0 && (
          <p className={styles.empty}>
            Ask for anything across your projects — to-dos, agents, an overview.
          </p>
        )}
        {messages.map((m) => (
          <div key={m.id} className={styles.message} data-role={m.role}>
            {m.role === "assistant" ? <Markdown>{m.text}</Markdown> : m.text}
          </div>
        ))}
        {busy && (
          <div className={styles.working} role="status">
            working…
          </div>
        )}
      </div>

      <form
        className={styles.composer}
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        <textarea
          className={styles.input}
          value={draft}
          rows={2}
          placeholder="Message the assistant…"
          aria-label="Message the assistant"
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            // Enter sends, Shift+Enter is a newline (chat convention).
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              void submit();
            }
          }}
        />
        <button
          type="submit"
          className={styles.sendBtn}
          aria-label="Send"
          title="Send"
          disabled={!draft.trim()}
        >
          <SendIcon />
        </button>
      </form>
    </aside>
  );
}
