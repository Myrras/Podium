/**
 * The assistant's Responses list, pinned above the chat log: the one-line
 * essentials of agent updates ("needs you" / "FYI") the assistant filed, so
 * they can be ticked off or answered without scrolling back through the
 * conversation. Workspace-wide, persisted by the backend, hidden when empty.
 */

import { useEffect, useRef, useState } from "react";

import type { ResponseItem } from "../ipc/types";
import { formatRelativeTime } from "../lib/dateFormat";
import { useAssistantStore } from "../state/assistantStore";
import { CaretIcon, CheckIcon, ReplyIcon, SendIcon } from "./icons";
import styles from "./ResponsesSection.module.css";

/**
 * A half-typed reply, filed under the item it answers — plus what it takes
 * to tell which newer item replaced that one, so the draft can follow it.
 */
interface Draft {
  itemId: string;
  kind: ResponseItem["kind"];
  processId: string | null;
  todoId: string | null;
  createdAt: string;
  text: string;
}

/**
 * Whether `newer` replaced the item `old` was filed under — the backend's
 * own rule (`ResponseItem::superseded_by`): same agent or same to-do, but an
 * FYI never replaces a "needs you", and an agent's item only goes to another
 * agent's item.
 */
function supersedes(newer: ResponseItem, old: Draft): boolean {
  if (Date.parse(newer.createdAt) <= Date.parse(old.createdAt)) return false;
  if (old.kind === "needsYou" && newer.kind === "fyi") return false;
  if (old.processId && !newer.processId) return false;
  return (
    (!!newer.processId && newer.processId === old.processId) ||
    (!!newer.todoId && newer.todoId === old.todoId)
  );
}

/**
 * Which row shows which draft: its own item's, or — when that item was
 * replaced — the newest item that replaced it. Two items side by side (an
 * FYI next to an open question about the same agent) never share one.
 */
function placeDrafts(
  items: ResponseItem[],
  drafts: Draft[],
): Map<string, Draft> {
  const placed = new Map<string, Draft>();
  const live = new Set(items.map((i) => i.id));
  for (const d of drafts) if (live.has(d.itemId)) placed.set(d.itemId, d);
  for (const d of drafts) {
    if (live.has(d.itemId)) continue;
    // Newest first, so this is the latest item that replaced it.
    const heir = items.find((i) => !placed.has(i.id) && supersedes(i, d));
    if (heir) placed.set(heir.id, d);
  }
  return placed;
}

export function ResponsesSection() {
  const responses = useAssistantStore((s) => s.responses);
  const [open, setOpen] = useState(true);
  const [drafts, setDrafts] = useState<Draft[]>([]);
  // One tick for the whole section re-renders every row's age.
  const [, setTick] = useState(0);
  useEffect(() => {
    const timer = setInterval(() => setTick((n) => n + 1), 60_000);
    return () => clearInterval(timer);
  }, []);

  const placed = placeDrafts(responses, drafts);

  function setDraftOf(item: ResponseItem, text: string | null) {
    const old = placed.get(item.id);
    setDrafts((prev) => {
      const rest = prev.filter((d) => d !== old);
      if (text === null) return rest;
      return [
        ...rest,
        {
          itemId: item.id,
          kind: item.kind,
          processId: item.processId,
          todoId: item.todoId,
          createdAt: item.createdAt,
          text,
        },
      ];
    });
  }

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
            <ResponseRow
              key={item.id}
              item={item}
              draft={placed.get(item.id)?.text ?? null}
              setDraft={(text) => setDraftOf(item, text)}
            />
          ))}
        </ul>
      )}
    </section>
  );
}

function ResponseRow({
  item,
  draft,
  setDraft,
}: {
  item: ResponseItem;
  /** The reply being typed, `null` while the reply box is closed. */
  draft: string | null;
  setDraft: (text: string | null) => void;
}) {
  const dismissResponse = useAssistantStore((s) => s.dismissResponse);
  const replyResponse = useAssistantStore((s) => s.replyResponse);
  // Focused only when the user opens the reply box — never on mount, or a
  // row that replaced one with an open draft would steal focus from
  // whatever the user is typing in now.
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const focusOnOpen = useRef(false);
  useEffect(() => {
    if (focusOnOpen.current && inputRef.current) {
      focusOnOpen.current = false;
      inputRef.current.focus();
    }
  });
  // Enter and the Send button both submit; ignore a second one while the
  // first is in flight.
  const [sending, setSending] = useState(false);
  // The parts `ResponseItem::label` is made of. Blank ones are dropped as it
  // does; the row's `white-space: nowrap` already collapses whitespace runs
  // (a newline in a name) the way it does per part.
  const agent = item.agent?.trim();
  const context = [item.project, item.todo, item.source].filter((p) =>
    p?.trim(),
  );

  async function send() {
    if (!draft?.trim() || sending) return;
    setSending(true);
    // Keep the draft when the reply is refused (the store logged why).
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
            {agent && <span className={styles.agent}>{agent}</span>}
            {context.length > 0 &&
              `${agent ? " · " : ""}${context.join(" · ")}`}
          </span>
          <span className={styles.kind} data-kind={item.kind}>
            {item.kind === "needsYou" ? "Needs you" : "FYI"}
          </span>
          <span className={styles.age} title={item.createdAt}>
            {formatRelativeTime(item.createdAt, new Date(), { compact: true })}
          </span>
        </div>
        <span className={styles.text}>{item.text}</span>
        {draft !== null && (
          <div className={styles.replyRow}>
            <textarea
              className={styles.replyInput}
              rows={2}
              ref={inputRef}
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
        onClick={() => {
          focusOnOpen.current = draft === null;
          setDraft(draft === null ? "" : null);
        }}
      >
        <ReplyIcon />
      </button>
      <button
        type="button"
        className={styles.iconBtn}
        aria-label="Mark as read"
        title="Mark as read"
        onClick={() => {
          setDraft(null);
          void dismissResponse(item.id);
        }}
      >
        <CheckIcon />
      </button>
    </li>
  );
}
