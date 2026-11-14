import { useLayoutEffect, useRef, useState } from "react";

import { type FloorState, useFloorStore } from "../../state/floorStore";
import { jumpTo } from "../../lib/floor/camera";
import { useRooms } from "../../lib/floor/rooms";
import { type Waypoint, waypoints } from "../../lib/floor/waypoints";
import { ProjectBadge } from "../ProjectBadge";
import styles from "./FloorWaypoints.module.css";

const keyOf = (marks: Waypoint[]) =>
  marks.map((m) => `${m.projectId}\t${m.needs}\t${m.crashed}`).join("\n");

function place(layer: HTMLElement, marks: Waypoint[], animate: boolean) {
  const els = new Map(
    [...layer.children].map((el) => [
      (el as HTMLElement).dataset.waypoint,
      el as HTMLElement,
    ]),
  );
  for (const m of marks) {
    const el = els.get(m.projectId);
    if (!el) continue;
    // A marker that has no place yet appears there; it never slides in from the corner.
    el.toggleAttribute("data-animate", animate && !!el.style.transform);
    el.style.transform = `translate(${m.x}px, ${m.y}px)`;
    el.style.setProperty("--angle", `${m.angle}rad`);
  }
}

export function FloorWaypoints() {
  const rooms = useRooms();
  const layer = useRef<HTMLDivElement>(null);
  // React renders only the set of markers and their counts; a pan moves them
  // outside React, as FloorMap moves the map.
  const [marks, setMarks] = useState<Waypoint[]>([]);
  // The latest computed markers (placed again once React renders a new set) and the set's key.
  const last = useRef({ marks, animate: false, key: "" });

  useLayoutEffect(() => {
    // shortcut: computed from store state, so during an animated jump the markers run one transition ahead of the picture.
    // Computed once per camera, viewport or rooms change; other floor state (hover, popover) is ignored.
    const sync = ({ camera, viewport, animate }: FloorState) => {
      const next = waypoints(rooms, camera, viewport);
      const key = keyOf(next);
      if (key !== last.current.key) setMarks(next);
      last.current = { marks: next, animate, key };
      place(layer.current!, next, animate);
    };
    sync(useFloorStore.getState());
    return useFloorStore.subscribe((s, p) => {
      if (s.camera !== p.camera || s.viewport !== p.viewport) sync(s);
    });
  }, [rooms]);
  // A new set renders new elements: give them their place before paint.
  useLayoutEffect(() => {
    const { marks, animate } = last.current;
    place(layer.current!, marks, animate);
  }, [marks]);

  return (
    <div ref={layer} className={styles.layer}>
      {marks.map((m) => {
        // Rooms render before the effect drops a marker of a removed project.
        const project = rooms.find(
          (r) => r.project.id === m.projectId,
        )?.project;
        if (!project) return null;
        const label = [
          m.needs > 0 &&
            `${m.needs} ${m.needs === 1 ? "agent" : "agents"} waiting on you`,
          m.crashed > 0 && `${m.crashed} crashed`,
        ]
          .filter(Boolean)
          .join(", ");
        return (
          <button
            key={m.projectId}
            type="button"
            className={styles.marker}
            data-waypoint={m.projectId}
            aria-label={`Go to ${project.name}: ${label}`}
            onClick={() => jumpTo(m.projectId)}
          >
            <span className={styles.shift}>
              <span className={styles.arrow} aria-hidden />
              {m.needs > 0 && (
                <span className={`${styles.badge} ${styles.needs}`}>
                  ?{m.needs > 1 ? m.needs : ""}
                </span>
              )}
              {m.crashed > 0 && (
                <span className={`${styles.badge} ${styles.crashed}`}>
                  !{m.crashed > 1 ? m.crashed : ""}
                </span>
              )}
              <ProjectBadge project={project} size={18} />
              <span className={styles.name}>{project.name}</span>
            </span>
          </button>
        );
      })}
    </div>
  );
}
