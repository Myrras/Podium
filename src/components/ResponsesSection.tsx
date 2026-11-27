/**
 * The assistant's Responses list, pinned above the chat log: the one-line
 * essentials of agent updates ("needs you" / "FYI") the assistant filed, so
 * they can be ticked off or answered without scrolling back through the
 * conversation. Workspace-wide, persisted by the backend, always shown: what
 * needs the user, and a collapsed group of parked items.
 */

import { useEffect, useState } from "react";

import type { ResponseItem } from "../ipc/types";
import { useAssistantStore } from "../state/assistantStore";
import { CaretIcon } from "./icons";
import { OverviewRow } from "./OverviewRow";
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
  const [parkedOpen, setParkedOpen] = useState(false);
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

  const needsYou = responses.filter((r) => r.state === "open");
  const parked = responses.filter((r) => r.state === "parked");

  const rows = (list: ResponseItem[]) => (
    <ul className={styles.list}>
      {list.map((item) => (
        <OverviewRow
          key={item.id}
          item={item}
          draft={placed.get(item.id)?.text ?? null}
          setDraft={(text) => setDraftOf(item, text)}
        />
      ))}
    </ul>
  );

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
        Needs you
        {needsYou.length > 0 && (
          <span className={styles.count}>{needsYou.length}</span>
        )}
      </button>
      {open &&
        (needsYou.length === 0 ? (
          <p className={styles.clear}>Nothing needs you.</p>
        ) : (
          rows(needsYou)
        ))}
      {parked.length > 0 && (
        <>
          <button
            type="button"
            className={`${styles.header} ${styles.parkedHeader}`}
            aria-expanded={parkedOpen}
            onClick={() => setParkedOpen(!parkedOpen)}
          >
            <span className={styles.caret} data-open={parkedOpen}>
              <CaretIcon />
            </span>
            Parked
            <span className={styles.plainCount}>{parked.length}</span>
          </button>
          {parkedOpen && rows(parked)}
        </>
      )}
    </section>
  );
}
