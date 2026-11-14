import { useLayoutEffect, useRef } from "react";

import { type FloorState, useFloorStore } from "../../state/floorStore";
import { jumpTo } from "../../lib/floor/camera";
import { useRooms } from "../../lib/floor/rooms";
import { waypoints } from "../../lib/floor/waypoints";
import { ProjectBadge } from "../ProjectBadge";
import styles from "./FloorWaypoints.module.css";

export function FloorWaypoints() {
  const rooms = useRooms();
  const layer = useRef<HTMLDivElement>(null);
  // React renders only the set of markers; a pan moves them outside React, as FloorMap moves the map.
  const key = useFloorStore((s) =>
    waypoints(rooms, s.camera, s.viewport)
      .map((m) => `${m.projectId}\t${m.needs}\t${m.crashed}`)
      .join("\n"),
  );
  // Read for the set and counts only (`key` re-renders on a change); positions come from `place`.
  const { camera, viewport } = useFloorStore.getState();
  const marks = waypoints(rooms, camera, viewport);

  useLayoutEffect(() => {
    // shortcut: computed from store state, so during an animated jump the markers run one transition ahead of the picture.
    const place = ({ camera, viewport, animate }: FloorState) => {
      const els = new Map(
        [...layer.current!.children].map((el) => [
          (el as HTMLElement).dataset.waypoint,
          el as HTMLElement,
        ]),
      );
      for (const m of waypoints(rooms, camera, viewport)) {
        const el = els.get(m.projectId);
        if (!el) continue;
        // A marker that has no place yet appears there; it never slides in from the corner.
        el.toggleAttribute("data-animate", animate && !!el.style.transform);
        el.style.transform = `translate(${m.x}px, ${m.y}px)`;
        el.style.setProperty("--angle", `${m.angle}rad`);
      }
    };
    place(useFloorStore.getState());
    return useFloorStore.subscribe((s, p) => {
      if (s.camera !== p.camera || s.viewport !== p.viewport) place(s);
    });
  }, [rooms, key]);

  return (
    <div ref={layer} className={styles.layer}>
      {marks.map((m) => {
        const project = rooms.find(
          (r) => r.project.id === m.projectId,
        )!.project;
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
