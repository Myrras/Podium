/**
 * Detail drawer for a Git board card, beside the board. The board
 * passes the live item (`undefined` once a refresh dropped it), so it shows a
 * "no longer on the board" note instead of stale content.
 */

import type { BoardItem, ProjectId } from "../ipc/types";
import { prRef } from "../lib/issueActions";
import { openExternalLink } from "../lib/links";
import { scrollEdges } from "../lib/scrollEdges";
import { moveTargets, useIssueStore } from "../state/issueStore";
import { CloseIcon, IssueIcon, PrIcon } from "./icons";
import { IssueActions } from "./IssueActions";
import styles from "./IssueDetailPane.module.css";
import { Markdown } from "./Markdown";

export function IssueDetailPane({
  projectId,
  item,
  itemRef,
  onClose,
}: {
  projectId: ProjectId;
  item: BoardItem | undefined;
  itemRef: string;
  onClose: () => void;
}) {
  const KindIcon = item?.kind === "pr" ? PrIcon : IssueIcon;
  const board = useIssueStore((s) => s.byProject[projectId]?.board);
  const move = useIssueStore((s) => s.move);
  const targets = item && board ? moveTargets(item, board.columns) : [];
  const provider = board?.provider ?? "github";
  const prWord = provider === "gitlab" ? "merge" : "pull";
  return (
    <aside className={styles.pane} aria-label={`${itemRef} details`}>
      <div className={styles.header}>
        <KindIcon className={styles.kindIcon} />
        <span className={styles.title}>
          {itemRef} {item?.title}
        </span>
        {item && (
          <IssueActions
            projectId={projectId}
            issue={item}
            itemRef={itemRef}
            className={styles.action}
          />
        )}
        <button
          type="button"
          className={styles.action}
          aria-label="Close details"
          title="Close"
          onClick={onClose}
        >
          <CloseIcon size={14} />
        </button>
      </div>
      {item ? (
        <div ref={scrollEdges} className={styles.content}>
          {(item.labels.length > 0 || item.draft || item.author) && (
            <ul className={styles.labels}>
              {item.draft && <li>draft</li>}
              {item.labels.map((l) => (
                <li key={l}>{l}</li>
              ))}
              {item.author && (
                <li className={styles.author}>by {item.author}</li>
              )}
            </ul>
          )}
          {item && targets.length > 0 && (
            <label className={styles.move}>
              Move to
              <select
                value=""
                onChange={(e) =>
                  e.target.value && void move(projectId, item, e.target.value)
                }
              >
                <option value="" disabled>
                  Choose column…
                </option>
                {targets.map((c) => (
                  <option key={c.id} value={c.id}>
                    {c.title}
                  </option>
                ))}
              </select>
            </label>
          )}
          {item.linked.length > 0 && (
            <section aria-label={`Linked ${prWord} requests`}>
              <h4 className={styles.linkedTitle}>Linked {prWord} requests</h4>
              <ul className={styles.linkedList}>
                {item.linked.map((pr) => (
                  <li key={pr.number}>
                    <button
                      type="button"
                      aria-label={`Open ${prRef(pr.number, provider)} in browser`}
                      onClick={() => openExternalLink(pr.url)}
                    >
                      <PrIcon size={12} />
                      {prRef(pr.number, provider)} {pr.title}
                      {pr.merged ? " (merged)" : pr.draft ? " (draft)" : ""}
                    </button>
                  </li>
                ))}
              </ul>
            </section>
          )}
          {item.body.trim() ? (
            <Markdown>{item.body}</Markdown>
          ) : (
            <p className={styles.muted}>No description.</p>
          )}
        </div>
      ) : (
        <p className={styles.muted}>This is no longer on the board.</p>
      )}
    </aside>
  );
}
