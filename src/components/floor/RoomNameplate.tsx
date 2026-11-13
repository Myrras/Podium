import { useRef, useState } from "react";
import type { KeyboardEvent, PointerEvent } from "react";

import { jumpTo } from "../../lib/floor/camera";
import { GRID, hits, snap } from "../../lib/floor/geometry";
import { getRooms } from "../../lib/floor/rooms";
import { startAgentIn } from "../../lib/floor/startAgent";
import type { RoomModel } from "../../lib/floor/types";
import { newTerminal } from "../../lib/terminals";
import { useFloorStore } from "../../state/floorStore";
import { useLayoutStore } from "../../state/layoutStore";
import { TerminalIcon } from "../icons";
import { ProjectBadge } from "../ProjectBadge";
import styles from "./RoomNameplate.module.css";

const CLICK_SLOP = 4;
const KEYS: Record<string, [number, number]> = {
  ArrowLeft: [-GRID, 0],
  ArrowRight: [GRID, 0],
  ArrowUp: [0, -GRID],
  ArrowDown: [0, GRID],
};

interface Drag {
  sx: number;
  sy: number;
  xy: [number, number];
  moved: boolean;
}

export function RoomNameplate({ room }: { room: RoomModel }) {
  const { project, rect } = room;
  const drag = useRef<Drag | null>(null);
  const root = useRef<HTMLDivElement>(null);
  const [bad, setBad] = useState(false);

  const blocked = (x: number, y: number) =>
    hits(
      { ...rect, x, y },
      getRooms()
        .filter((r) => r.project.id !== project.id)
        .map((r) => r.rect),
    );
  // The room's positioned root moves while dragging.
  const shift = (x: number, y: number) => {
    const el = root.current?.closest<HTMLElement>("[data-room]");
    if (el)
      el.style.transform =
        x === rect.x && y === rect.y
          ? ""
          : `translate(${x - rect.x}px, ${y - rect.y}px)`;
  };

  const onDown = (e: PointerEvent<HTMLButtonElement>) => {
    if (e.button !== 0) return;
    e.currentTarget.setPointerCapture?.(e.pointerId);
    drag.current = {
      sx: e.clientX,
      sy: e.clientY,
      xy: [rect.x, rect.y],
      moved: false,
    };
  };
  const onMove = (e: PointerEvent<HTMLButtonElement>) => {
    const d = drag.current;
    if (!d) return;
    const dx = e.clientX - d.sx;
    const dy = e.clientY - d.sy;
    if (!d.moved && Math.hypot(dx, dy) < CLICK_SLOP) return;
    d.moved = true;
    const z = useFloorStore.getState().camera.z || 1;
    d.xy = snap([rect.x + dx / z, rect.y + dy / z]);
    shift(...d.xy);
    setBad(blocked(...d.xy));
  };
  const onUp = () => {
    const d = drag.current;
    if (!d?.moved) return; // a plain click is handled by onClick
    const [x, y] = d.xy;
    if (!blocked(x, y))
      useLayoutStore.getState().setFloorPosition(project.root, [x, y]);
    shift(rect.x, rect.y);
    setBad(false);
  };
  const onClick = () => {
    const moved = drag.current?.moved;
    drag.current = null;
    if (!moved) jumpTo(project.id);
  };
  const onKey = (e: KeyboardEvent<HTMLButtonElement>) => {
    const step = KEYS[e.key];
    if (!step) return;
    e.preventDefault();
    const x = rect.x + step[0];
    const y = rect.y + step[1];
    if (!blocked(x, y))
      useLayoutStore.getState().setFloorPosition(project.root, [x, y]);
  };

  return (
    <div ref={root} className={styles.plate} data-bad={bad || undefined}>
      <button
        type="button"
        className={styles.grip}
        data-no-pan
        onPointerDown={onDown}
        onPointerMove={onMove}
        onPointerUp={onUp}
        onPointerCancel={onUp}
        onClick={onClick}
        onKeyDown={onKey}
      >
        <ProjectBadge project={project} size={16} />
        <span className={styles.name} title={project.name}>
          {project.name}
        </span>
      </button>
      {room.needs > 0 && (
        <span className={styles.needs} aria-label={`${room.needs} need you`}>
          ? {room.needs}
        </span>
      )}
      {room.crashed > 0 && (
        <span className={styles.crashed} aria-label={`${room.crashed} crashed`}>
          ! {room.crashed}
        </span>
      )}
      <button
        type="button"
        className={styles.btn}
        aria-label="New agent"
        onClick={(e) => startAgentIn(project, e)}
      >
        +
      </button>
      {project.sections.terminals && (
        <button
          type="button"
          className={styles.btn}
          aria-label={`New terminal in ${project.name}`}
          title="New terminal"
          onClick={() => void newTerminal(project)}
        >
          <TerminalIcon />
        </button>
      )}
      <button
        type="button"
        className={styles.btn}
        aria-label="Project menu"
        data-fixture={`${project.id}:menu`}
        onClick={() =>
          useFloorStore
            .getState()
            .openPopover({ projectId: project.id, kind: "menu", itemId: null })
        }
      >
        ⋯
      </button>
    </div>
  );
}
