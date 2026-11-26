/**
 * Floor mode: the work island as a map of project rooms. Fills the island in
 * place of the sidebar and work area; App keeps every subscription mounted.
 */

import { useEffect, useLayoutEffect, useState } from "react";

import { useFloorStore } from "../../state/floorStore";
import { useLayoutStore } from "../../state/layoutStore";
import { FloorAgentModals } from "./FloorAgentModals";
import { FloorBoardModal } from "./FloorBoardModal";
import { FloorHeader } from "./FloorHeader";
import { FloorMap } from "./FloorMap";
import { FloorPopovers } from "./FloorPopovers";
import { FloorStatusBar } from "./FloorStatusBar";
import { FloorTerminalModal } from "./FloorTerminalModal";
import { FloorWaypoints } from "./FloorWaypoints";
import styles from "./Floor.module.css";

export function Floor() {
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
      <div className={styles.mapArea}>
        <FloorMap />
        <FloorWaypoints />
      </div>
      <FloorStatusBar />
      <FloorPopovers />
      <FloorTerminalModal />
      <FloorBoardModal />
      <FloorAgentModals />
    </>
  );
}
