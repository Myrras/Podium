/**
 * Floor mode: the work island as a map of project rooms. Fills the island in
 * place of the sidebar and work area; App keeps every subscription mounted.
 */

import { useEffect, useLayoutEffect, useState } from "react";

import type { ProjectId } from "../../ipc/types";
import { useFloorStore } from "../../state/floorStore";
import { useIssueStore } from "../../state/issueStore";
import { useLayoutStore } from "../../state/layoutStore";
import { GitBoard } from "../GitBoard";
import { FloorAgentModals } from "./FloorAgentModals";
import { FloorHeader } from "./FloorHeader";
import { FloorMap } from "./FloorMap";
import { FloorPopovers } from "./FloorPopovers";
import { FloorStatusBar } from "./FloorStatusBar";
import { FloorTerminalModal } from "./FloorTerminalModal";
import { FloorWaypoints } from "./FloorWaypoints";
import styles from "./Floor.module.css";

/** Keeps the open board fresh, as GitButton does in Project mode (no GitButton is mounted here). */
function BoardRefresh({ projectId }: { projectId: ProjectId }) {
  const startAutoRefresh = useIssueStore((s) => s.startAutoRefresh);
  const loadOverview = useIssueStore((s) => s.loadOverview);
  useEffect(() => startAutoRefresh(projectId), [projectId, startAutoRefresh]);
  useEffect(() => void loadOverview(), [loadOverview]);
  return null;
}

export function Floor() {
  const openBoard = useLayoutStore((s) => s.openBoard);

  useEffect(() => () => useFloorStore.getState().reset(), []);
  // A board or sub-agent left open in Project mode must not open over a fresh
  // floor: cleared before the first paint, with nothing mounted until then.
  const [ready, setReady] = useState(false);
  useLayoutEffect(() => {
    const layout = useLayoutStore.getState();
    layout.clearOpenBoard();
    layout.clearOpenSubagent();
    setReady(true);
  }, []);
  if (!ready) return null;

  return (
    <>
      <FloorHeader />
      {openBoard ? (
        <>
          <BoardRefresh projectId={openBoard.projectId} />
          <GitBoard key={openBoard.projectId} projectId={openBoard.projectId} />
        </>
      ) : (
        <div className={styles.mapArea}>
          <FloorMap />
          <FloorWaypoints />
        </div>
      )}
      <FloorStatusBar />
      <FloorPopovers />
      <FloorTerminalModal />
      <FloorAgentModals />
    </>
  );
}
