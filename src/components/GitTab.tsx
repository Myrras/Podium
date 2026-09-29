/**
 * The sidebar's Git tab for one project: a single row that opens the project's
 * Git board in the work area, with a short summary of it. Mounting it keeps
 * the board fresh (`startAutoRefresh`), so the summary is live while the tab
 * is shown and the board is already loaded when opened.
 */

import { useEffect } from "react";

import type { ProjectId } from "../ipc/types";
import { boardErrorText } from "../lib/issueActions";
import { useIssueStore } from "../state/issueStore";
import { useLayoutStore } from "../state/layoutStore";
import { BoardIcon } from "./icons";
import styles from "./Sidebar.module.css";

export function GitTab({ projectId }: { projectId: ProjectId }) {
  const entry = useIssueStore((s) => s.byProject[projectId]);
  const startAutoRefresh = useIssueStore((s) => s.startAutoRefresh);
  const loadOverview = useIssueStore((s) => s.loadOverview);
  const isOpen = useLayoutStore((s) => s.openBoard?.projectId === projectId);
  const openBoard = useLayoutStore((s) => s.openBoardInWorkArea);

  useEffect(() => startAutoRefresh(projectId), [projectId, startAutoRefresh]);
  useEffect(() => void loadOverview(), [loadOverview]);

  const items = entry?.board?.items ?? [];
  const open = items.filter(
    (i) => i.column === "open" || i.column === "inProgress",
  ).length;
  const review = items.filter((i) => i.column === "review").length;

  return (
    <div className={styles.subsection}>
      <button
        type="button"
        className={`${styles.sectionHeader} ${styles.gitTab}`}
        data-open={isOpen ? "true" : undefined}
        onClick={() => openBoard(projectId)}
      >
        <BoardIcon className={styles.panelIcon} />
        <span className={styles.panelTitle}>Git</span>
        {entry?.error ? (
          <span
            className={styles.gitError}
            title={boardErrorText(entry.error)}
            aria-label={boardErrorText(entry.error)}
          >
            !
          </span>
        ) : (
          entry?.board && (
            <span className={styles.gitSummary}>
              {open} open · {review} review
            </span>
          )
        )}
      </button>
    </div>
  );
}
