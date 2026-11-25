/**
 * One row of the assistant's overview ("Needs you" / "Parked"): the agent's
 * own bot on a pad, a mark and a state word that agree with its pose, and the
 * item's text. Clicking the row goes to the agent; the actions take the
 * age's place on hover or focus.
 */

import { useEffect, useRef, useState, type CSSProperties } from "react";

import type { ResponseItem, ResponseReason } from "../ipc/types";
import { formatRelativeTime } from "../lib/dateFormat";
import { showAgent } from "../lib/floor/camera";
import { roomOf, useRooms } from "../lib/floor/rooms";
import { noteSheet } from "../lib/floor/sprites";
import type { DeskState } from "../lib/floor/types";
import { useAssistantStore } from "../state/assistantStore";
import { useLayoutStore } from "../state/layoutStore";
import { useProcessStore } from "../state/processStore";
import { Bot } from "./floor/Bot";
import {
  BringBackIcon,
  CheckIcon,
  ParkIcon,
  ReplyIcon,
  SendIcon,
} from "./icons";
import styles from "./OverviewRow.module.css";

// Pose, mark and word per reason: state is never colour alone.
// Review and other rest on frame 0 ("working" with its typing stopped in CSS).
export const LOOK: Record<
  ResponseReason,
  { pose: DeskState; mark?: string; word?: string }
> = {
  question: { pose: "needs", mark: "?", word: "Asks" },
  crashed: { pose: "crashed", mark: "!", word: "Crashed" },
  review: { pose: "working", mark: "✓", word: "Ready to review" },
  message: { pose: "working" },
  other: { pose: "working" },
};

const trimmed = (s: string | null) => s?.trim() || null;

/** Who the item is from; blank name parts are dropped, as `ResponseItem::label` does. */
export function itemName(item: ResponseItem): string {
  return (
    (item.reason === "message" ? trimmed(item.sender) : trimmed(item.agent)) ??
    trimmed(item.todo) ??
    trimmed(item.project) ??
    trimmed(item.source) ??
    "Assistant"
  );
}

/** Where the item comes from: the source for messages, else the project. */
export function itemPlace(item: ResponseItem): string | null {
  return item.reason === "message"
    ? trimmed(item.source)
    : (trimmed(item.project) ?? trimmed(item.source));
}

/** The agent's bot on a state pad with its mark, or the note sprite for a message. */
export function OverviewAvatar({
  item,
  size = 32,
}: {
  item: ResponseItem;
  size?: number;
}) {
  const rooms = useRooms();
  const parked = item.state === "parked";
  const look = LOOK[item.reason];
  const variant = item.processId
    ? (roomOf(item.processId, rooms)?.variants[item.processId] ?? 0)
    : 0;
  return (
    <span
      className={styles.pad}
      data-reason={item.reason}
      data-parked={parked || undefined}
    >
      {item.reason === "message" ? (
        <span className={styles.note} aria-hidden="true">
          <i style={{ backgroundImage: noteSheet() }} />
        </span>
      ) : (
        <Bot
          color={item.agentColor}
          state={parked ? "idle" : look.pose}
          seed={item.processId ?? item.key}
          size={size}
          variant={variant}
        />
      )}
      {!parked && look.mark && (
        <b className={styles.mark} aria-hidden="true">
          {look.mark}
        </b>
      )}
    </span>
  );
}

export function OverviewRow({
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
  const parkResponse = useAssistantStore((s) => s.parkResponse);
  const unparkResponse = useAssistantStore((s) => s.unparkResponse);
  const replyResponse = useAssistantStore((s) => s.replyResponse);
  const onFloor = useLayoutStore((s) => s.mode === "floor");
  // A removed agent has nowhere to go.
  const hasProcess = useProcessStore((s) =>
    s.processes.some((p) => p.id === item.processId),
  );
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

  const parked = item.state === "parked";
  const isMessage = item.reason === "message";
  const look = LOOK[item.reason];
  const name = itemName(item);
  const place = itemPlace(item);
  const age = formatRelativeTime(item.createdAt, new Date(), {
    compact: true,
  });
  const processId = hasProcess ? item.processId : null;

  function go() {
    if (!processId) return;
    if (onFloor) showAgent(processId);
    else useProcessStore.getState().setActiveProcess(processId);
  }

  async function send() {
    if (!draft?.trim() || sending) return;
    setSending(true);
    // Keep the draft when the reply is refused (the store logged why).
    const sent = await replyResponse(item.id, draft);
    setSending(false);
    if (sent) setDraft(null);
  }

  const avatar = <OverviewAvatar item={item} size={parked ? 16 : 32} />;

  const dismiss = (
    <button
      type="button"
      className={styles.iconBtn}
      aria-label="Dismiss"
      title="Dismiss"
      onClick={() => {
        setDraft(null);
        void dismissResponse(item.key);
      }}
    >
      <CheckIcon />
    </button>
  );

  const actions = parked ? (
    <>
      <button
        type="button"
        className={styles.iconBtn}
        aria-label="Bring back"
        title="Bring back"
        onClick={() => void unparkResponse(item.key)}
      >
        <BringBackIcon />
      </button>
      {dismiss}
    </>
  ) : (
    <>
      {!isMessage && (
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
      )}
      <button
        type="button"
        className={styles.iconBtn}
        aria-label="Park until it changes"
        title="Park until it changes"
        onClick={() => {
          setDraft(null);
          void parkResponse(item.key);
        }}
      >
        <ParkIcon />
      </button>
      {dismiss}
    </>
  );
  const actionCount = parked || isMessage ? 2 : 3;

  return (
    <li
      className={styles.row}
      data-reason={item.reason}
      data-parked={parked || undefined}
      style={{ "--n": actionCount } as CSSProperties}
      tabIndex={processId ? 0 : undefined}
      onClick={(e) => {
        // The row's own controls and a text selection are not "go".
        if ((e.target as Element).closest("button, textarea")) return;
        if (window.getSelection()?.toString()) return;
        go();
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter" && e.target === e.currentTarget) go();
      }}
    >
      {avatar}
      <div className={styles.body}>
        <div className={styles.l1}>
          <span className={styles.who}>{name}</span>
          <span className={styles.age} title={item.createdAt}>
            {parked ? `parked ${age}` : age}
          </span>
        </div>
        {!parked && (look.word || (place && place !== name)) && (
          <div className={styles.l2}>
            {look.word ? (
              <>
                <b>{look.word}</b>
                {place && ` in ${place}`}
              </>
            ) : (
              place
            )}
          </div>
        )}
        {!parked && <p className={styles.text}>{item.text}</p>}
        {draft !== null && (
          <div className={styles.replyRow}>
            <textarea
              className={styles.replyInput}
              rows={2}
              ref={inputRef}
              value={draft}
              aria-label="Reply text"
              placeholder={`Reply to ${name}`}
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !e.shiftKey) {
                  e.preventDefault();
                  void send();
                } else if (e.key === "Escape") {
                  e.preventDefault(); // keep the enclosing popover open
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
      <div className={styles.acts}>{actions}</div>
    </li>
  );
}
