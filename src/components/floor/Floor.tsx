/**
 * Floor mode: the work island as a map of project rooms. Fills the island in
 * place of the sidebar and work area; App keeps every subscription mounted.
 */

import { useEffect } from "react";

import { useFloorStore } from "../../state/floorStore";
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

export function Floor() {
  const openBoard = useLayoutStore((s) => s.openBoard);

  useEffect(() => () => useFloorStore.getState().reset(), []);

  return (
    <>
      <FloorHeader />
      {openBoard ? (
        <GitBoard key={openBoard.projectId} projectId={openBoard.projectId} />
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
