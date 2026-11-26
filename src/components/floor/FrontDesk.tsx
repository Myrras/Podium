/**
 * The assistant on the floor: a desk pinned to the bottom-right corner of the
 * map (it does not pan or zoom). The house bot opens the chat, the queue holds
 * every open Responses item, and the count opens the full list; both open as
 * popovers above the desk (`FrontDeskPopovers`, anchored on `data-desk`).
 */

import { useEffect, useState } from "react";

import type { ResponseItem } from "../../ipc/types";
import { formatRelativeTime } from "../../lib/dateFormat";
import { showAgent } from "../../lib/floor/camera";
import { useAssistantStore } from "../../state/assistantStore";
import { useFloorStore } from "../../state/floorStore";
import { AssistantDesk, deskLine, useAssistantDesk } from "../AssistantDesk";
import { LOOK, OverviewAvatar, itemName, itemPlace } from "../OverviewRow";
import { CheckIcon, ParkIcon, ReplyIcon, SendIcon } from "../icons";
import styles from "./FrontDesk.module.css";

const MAX_QUEUE = 6;

export function FrontDesk({
  onOpenChat,
  onOpenList,
}: {
  onOpenChat?: () => void;
  onOpenList?: () => void;
}) {
  const responses = useAssistantStore((s) => s.responses);
  const { state, helpers } = useAssistantDesk();
  const desk = useFloorStore((s) => s.deskPopover);
  const open = responses
    .filter((r) => r.state === "open")
    .sort((a, b) => Date.parse(b.createdAt) - Date.parse(a.createdAt));
  const n = open.length;

  return (
    <div className={styles.fd} data-pop={desk ?? undefined}>
      <div className={styles.plate}>Front desk</div>
      <div className={styles.top}>
        <button
          type="button"
          className={styles.bot}
          aria-label="Open chat"
          title="Open chat"
          data-desk="chat"
          aria-expanded={desk === "chat"}
          onClick={onOpenChat}
        >
          <AssistantDesk state={state} helpers={helpers} size={48} hideLine />
        </button>
        <div className={styles.queue}>
          {open.slice(0, MAX_QUEUE).map((item) => (
            <Figure key={item.key} item={item} />
          ))}
          {n > MAX_QUEUE && (
            <span className={styles.more}>+{n - MAX_QUEUE}</span>
          )}
        </div>
      </div>
      <div className={styles.line}>
        <button
          type="button"
          className={styles.count}
          data-n={n}
          data-desk="list"
          aria-expanded={desk === "list"}
          onClick={onOpenList}
        >
          {n ? `${n} ${n === 1 ? "needs" : "need"} you` : "Nothing needs you"}
        </button>
        <span className={styles.state} title={deskLine(state, helpers)}>
          {deskLine(state, helpers)}
        </span>
      </div>
    </div>
  );
}

/** One queued item; its card shows on hover or keyboard focus (CSS). */
function Figure({ item }: { item: ResponseItem }) {
  const parkResponse = useAssistantStore((s) => s.parkResponse);
  const dismissResponse = useAssistantStore((s) => s.dismissResponse);
  const replyResponse = useAssistantStore((s) => s.replyResponse);
  const [draft, setDraft] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const pid = item.processId;
  const name = itemName(item);
  const place = itemPlace(item);
  const word = LOOK[item.reason].word;

  // Rings the agent's desk on the map while hovered or focused.
  const ring = (on: boolean) => {
    const floor = useFloorStore.getState();
    if (on) floor.setHoverDesk(pid);
    else if (floor.hoverDesk === pid) floor.setHoverDesk(null);
  };
  // A figure that leaves while hovered (parked, dismissed) drops its ring.
  useEffect(
    () => () => {
      const floor = useFloorStore.getState();
      if (pid && floor.hoverDesk === pid) floor.setHoverDesk(null);
    },
    [pid],
  );

  async function send() {
    if (!draft?.trim() || sending) return;
    setSending(true);
    const sent = await replyResponse(item.id, draft);
    setSending(false);
    if (sent) setDraft(null);
  }

  return (
    <span
      className={styles.q}
      data-reason={item.reason}
      onMouseEnter={() => pid && ring(true)}
      onMouseLeave={() => pid && ring(false)}
      onFocus={() => pid && ring(true)}
      onBlur={(e) => {
        if (pid && !e.currentTarget.contains(e.relatedTarget as Node | null))
          ring(false);
      }}
    >
      <button
        type="button"
        className={styles.fig}
        aria-label={name}
        onClick={() => {
          if (!pid) return;
          showAgent(pid);
          // Opening the agent handles Podium's item about it (see OverviewRow).
          if (item.key.startsWith("agent:")) void dismissResponse(item.key);
        }}
      >
        <OverviewAvatar item={item} size={32} />
      </button>
      <div className={styles.card}>
        <div className={styles.cardIn}>
          <div className={styles.l1}>
            <span className={styles.who}>{name}</span>
            <span className={styles.age} title={item.createdAt}>
              {formatRelativeTime(item.createdAt, new Date(), {
                compact: true,
              })}
            </span>
          </div>
          {(word || (place && place !== name)) && (
            <div className={styles.l2}>
              {word ? (
                <>
                  <b>{word}</b>
                  {place && ` in ${place}`}
                </>
              ) : (
                place
              )}
            </div>
          )}
          <p className={styles.text}>{item.text}</p>
          {draft !== null && (
            <div className={styles.replyRow}>
              <textarea
                className={styles.replyInput}
                rows={2}
                autoFocus
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
                className={styles.iconBtn}
                aria-label="Send reply"
                title="Send reply"
                disabled={!draft.trim() || sending}
                onClick={() => void send()}
              >
                <SendIcon />
              </button>
            </div>
          )}
          <div className={styles.acts}>
            {item.reason !== "message" && (
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
            )}
            <button
              type="button"
              className={styles.iconBtn}
              aria-label="Park until it changes"
              title="Park until it changes"
              onClick={() => void parkResponse(item.key)}
            >
              <ParkIcon />
            </button>
            <button
              type="button"
              className={styles.iconBtn}
              aria-label="Dismiss"
              title="Dismiss"
              onClick={() => void dismissResponse(item.key)}
            >
              <CheckIcon />
            </button>
          </div>
        </div>
      </div>
    </span>
  );
}
