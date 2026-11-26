/**
 * The Git board in an overlay over the floor, like the terminal modal. Not
 * `Modal`: the card is resizable and must not scale.
 */

import { useEffect, useRef } from "react";
import { createPortal } from "react-dom";

import type { ProjectId } from "../../ipc/types";
import { useSavedSize } from "../../lib/useSavedSize";
import { useIssueStore } from "../../state/issueStore";
import { useLayoutStore } from "../../state/layoutStore";
import { GitBoard } from "../GitBoard";
import { blurWithin } from "../Popover";
import { trapTab } from "./trapTab";
import styles from "./FloorBoardModal.module.css";

/** Keeps the open board fresh, as GitButton does in Project mode (no GitButton is mounted here). */
function BoardRefresh({ projectId }: { projectId: ProjectId }) {
  const startAutoRefresh = useIssueStore((s) => s.startAutoRefresh);
  const loadOverview = useIssueStore((s) => s.loadOverview);
  useEffect(() => startAutoRefresh(projectId), [projectId, startAutoRefresh]);
  useEffect(() => void loadOverview(), [loadOverview]);
  return null;
}

export function FloorBoardModal() {
  const openBoard = useLayoutStore((s) => s.openBoard);
  const clearOpenBoard = useLayoutStore((s) => s.clearOpenBoard);
  const overlay = useRef<HTMLDivElement>(null);
  const sized = useSavedSize("board");
  if (!openBoard) return null;

  const close = () => {
    blurWithin(overlay.current);
    clearOpenBoard();
  };

  return createPortal(
    <div
      ref={overlay}
      className={styles.overlay}
      role="presentation"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) close();
      }}
    >
      <div
        ref={sized}
        className={styles.card}
        role="dialog"
        aria-modal="true"
        aria-label="Git board"
        onKeyDown={(e) => {
          trapTab(e);
          // The search field handles (and prevents) its own Esc.
          if (e.key === "Escape" && !e.defaultPrevented) close();
        }}
      >
        <BoardRefresh projectId={openBoard.projectId} />
        <GitBoard key={openBoard.projectId} projectId={openBoard.projectId} />
      </div>
    </div>,
    document.body,
  );
}
