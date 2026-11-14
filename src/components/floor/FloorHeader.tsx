import type { KeyboardEvent } from "react";

import { jumpTo, zoomBy } from "../../lib/floor/camera";
import { deskState, useRooms } from "../../lib/floor/rooms";
import { scrollEdges } from "../../lib/scrollEdges";
import { UNTRACKED_ACTIVITY } from "../../state/agentActivityStore";
import { useAgentActivityStore } from "../../state/agentActivityStore";
import { useFloorStore } from "../../state/floorStore";
import { useLayoutStore } from "../../state/layoutStore";
import { useProjectStore } from "../../state/projectStore";
import { FitIcon, ZoomInIcon, ZoomOutIcon } from "../icons";
import { ProjectBadge } from "../ProjectBadge";
import styles from "./FloorHeader.module.css";

export function FloorHeader() {
  const rooms = useRooms();
  const details = useAgentActivityStore((s) => s.details);
  const focus = useFloorStore((s) => s.focus);
  const blocked = useFloorStore(
    (s) => s.popover !== null || s.terminalId !== null,
  );
  const openProjectDialog = useProjectStore((s) => s.openProjectDialog);

  const needs = rooms.reduce((n, r) => n + r.needs, 0);
  const crashed = rooms.reduce((n, r) => n + r.crashed, 0);
  const working = rooms.reduce(
    (n, r) =>
      n +
      r.agents.filter(
        (a) => deskState(a, details[a.id] ?? UNTRACKED_ACTIVITY) === "working",
      ).length,
    0,
  );

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Escape" && !blocked) jumpTo("all");
  };
  const addProject = () => void openProjectDialog();

  return (
    <header className={styles.header} onKeyDown={onKeyDown}>
      <div ref={scrollEdges} className={styles.chips}>
        {rooms.length === 0 ? (
          <span className={styles.empty}>
            No projects yet. Add one and it gets a room on the floor.
          </span>
        ) : (
          <button
            type="button"
            className={styles.chip}
            aria-pressed={focus === "all"}
            onClick={() => jumpTo("all")}
          >
            All projects
          </button>
        )}
        {rooms.map((r) => (
          <button
            key={r.project.id}
            type="button"
            className={styles.chip}
            aria-pressed={focus === r.project.id}
            onClick={() => jumpTo(r.project.id)}
          >
            <ProjectBadge project={r.project} size={16} />
            {r.project.name}
            {r.needs > 0 && (
              <span className={styles.needs} aria-label={`${r.needs} need you`}>
                ? {r.needs}
              </span>
            )}
            {r.crashed > 0 && (
              <span
                className={styles.crashed}
                aria-label={`${r.crashed} crashed`}
              >
                ! {r.crashed}
              </span>
            )}
          </button>
        ))}
        <button
          type="button"
          className={`${styles.chip} ${styles.add}`}
          onClick={addProject}
        >
          Add project…
        </button>
      </div>
      <div className={styles.totals}>
        {needs > 0 && <span className={styles.needs}>{needs} need you</span>}
        {crashed > 0 && (
          <span className={styles.crashed}>{crashed} crashed</span>
        )}
        {working > 0 && <span>{working} working</span>}
      </div>
      <div className={styles.buttons} data-popover-keep>
        <button
          type="button"
          aria-label="Zoom out"
          title="Zoom out"
          onClick={() => zoomBy(0.8)}
        >
          <ZoomOutIcon />
        </button>
        <button
          type="button"
          aria-label="Zoom in"
          title="Zoom in"
          onClick={() => zoomBy(1.25)}
        >
          <ZoomInIcon />
        </button>
        <button
          type="button"
          aria-label="Fit all"
          title="Fit all"
          onClick={() => jumpTo("all")}
        >
          <FitIcon />
        </button>
        <button
          type="button"
          onClick={() => {
            useLayoutStore.getState().resetFloorPositions();
            jumpTo("all");
          }}
        >
          Reset layout
        </button>
      </div>
    </header>
  );
}
