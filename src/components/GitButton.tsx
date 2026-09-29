/**
 * The project header's Git button: opens the project's Git board in the work
 * area. Unlike the other header actions it stays visible at rest, so the board
 * is one click away, and it carries the board's state — a tooltip summary, an
 * accent while the board is open, a warning dot when the last fetch failed.
 * Mounting it keeps the board fresh (`startAutoRefresh`), so the board is
 * already loaded when opened.
 */

import { useEffect } from "react";

import type { ProjectId } from "../ipc/types";
import { boardErrorText } from "../lib/issueActions";
import { useIssueStore } from "../state/issueStore";
import { useLayoutStore } from "../state/layoutStore";
import { useProjectStore } from "../state/projectStore";
import { BoardIcon } from "./icons";
import styles from "./Sidebar.module.css";

export function GitButton({ projectId }: { projectId: ProjectId }) {
  const entry = useIssueStore((s) => s.byProject[projectId]);
  const startAutoRefresh = useIssueStore((s) => s.startAutoRefresh);
  const loadOverview = useIssueStore((s) => s.loadOverview);
  const isOpen = useLayoutStore((s) => s.openBoard?.projectId === projectId);
  const openBoard = useLayoutStore((s) => s.openBoardInWorkArea);
  const setActiveProject = useProjectStore((s) => s.setActiveProject);

  useEffect(() => startAutoRefresh(projectId), [projectId, startAutoRefresh]);
  useEffect(() => void loadOverview(), [loadOverview]);

  const items = entry?.board?.items ?? [];
  const open = items.filter((i) => i.column !== "done").length;
  const title = entry?.error
    ? `Git board — ${boardErrorText(entry.error)}`
    : entry?.board
      ? `Git board — ${open} open`
      : "Git board";

  return (
    <button
      type="button"
      className={`${styles.projectAction} ${styles.gitButton}`}
      data-open={isOpen ? "true" : undefined}
      data-error={entry?.error ? "true" : undefined}
      aria-label={title}
      title={title}
      onClick={(e) => {
        e.stopPropagation();
        setActiveProject(projectId);
        openBoard(projectId);
      }}
    >
      <BoardIcon size={12} />
    </button>
  );
}
