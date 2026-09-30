/**
 * The assistant's Responses list, pinned above the chat log: the one-line
 * essentials of agent updates ("needs you" / "FYI") the assistant filed, so
 * they can be ticked off or answered without scrolling back through the
 * conversation. Workspace-wide, persisted by the backend, hidden when empty.
 */

import { useState } from "react";

import type { ResponseItem } from "../ipc/types";
import { shortAge } from "../lib/dateFormat";
import { useAssistantStore } from "../state/assistantStore";
import { CaretIcon, CheckIcon, ReplyIcon, SendIcon } from "./icons";
import styles from "./ResponsesSection.module.css";

export function ResponsesSection() {
  const responses = useAssistantStore((s) => s.responses);
  const [open, setOpen] = useState(true);

  if (responses.length === 0) return null;

  return (
    <section className={styles.section} aria-label="Responses">
      <button
        type="button"
        className={styles.header}
        aria-expanded={open}
        onClick={() => setOpen(!open)}
      >
        <span className={styles.caret} data-open={open}>
          <CaretIcon />
        </span>
        Responses
        <span className={styles.count}>{responses.length}</span>
      </button>
      {open && (
        <ul className={styles.list}>
          {responses.map((item) => (
            <ResponseRow key={item.id} item={item} />
          ))}
        </ul>
      )}
    </section>
  );
}

function ResponseRow({ item }: { item: ResponseItem }) {
  const dismissResponse = useAssistantStore((s) => s.dismissResponse);
  const replyResponse = useAssistantStore((s) => s.replyResponse);
  const [draft, setDraft] = useState<string | null>(null);
  // Enter and the Send button both submit; ignore a second one while the
  // first is in flight.
  const [sending, setSending] = useState(false);
  const context = [item.project, item.todo, item.source].filter(Boolean);

  async function send() {
    if (!draft?.trim() || sending) return;
    setSending(true);
    // Keep the draft when the reply is refused (the store logged why). A send
    // that fails after the backend took the item off the list remounts the
    // row and loses it — rare: only a session that cannot spawn gets there.
    const sent = await replyResponse(item.id, draft);
    setSending(false);
    if (sent) setDraft(null);
  }

  return (
    <li className={styles.row}>
      <span
        className={styles.dot}
        style={
          item.agentColor
            ? ({ "--session-color": item.agentColor } as React.CSSProperties)
            : undefined
        }
      />
      <div className={styles.body}>
        <div className={styles.meta}>
          <span className={styles.who}>
            {item.agent && <span className={styles.agent}>{item.agent}</span>}
            {context.length > 0 &&
              `${item.agent ? " · " : ""}${context.join(" · ")}`}
          </span>
          <span className={styles.kind} data-kind={item.kind}>
            {item.kind === "needsYou" ? "Needs you" : "FYI"}
          </span>
          <span className={styles.age} title={item.createdAt}>
            {shortAge(item.createdAt)}
          </span>
        </div>
        <span className={styles.text}>{item.text}</span>
        {draft !== null && (
          <div className={styles.replyRow}>
            <textarea
              className={styles.replyInput}
              rows={2}
              autoFocus
              value={draft}
              aria-label="Reply text"
              placeholder="Reply to the assistant…"
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !e.shiftKey) {
                  e.preventDefault();
                  void send();
                } else if (e.key === "Escape") {
                  setDraft(null);
                }
              }}
            />
            <button
              type="button"
              className={styles.sendBtn}
              aria-label="Send reply"
              title="Send reply"
              disabled={!draft.trim() || sending}
              onClick={() => void send()}
            >
              <SendIcon />
            </button>
          </div>
        )}
      </div>
      <button
        type="button"
        className={styles.iconBtn}
        aria-label="Reply"
        title="Reply"
        aria-pressed={draft !== null}
        onClick={() => setDraft(draft === null ? "" : null)}
      >
        <ReplyIcon />
      </button>
      <button
        type="button"
        className={styles.iconBtn}
        aria-label="Mark as read"
        title="Mark as read"
        onClick={() => void dismissResponse(item.id)}
      >
        <CheckIcon />
      </button>
    </li>
  );
}
