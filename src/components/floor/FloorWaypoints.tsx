import { useFloorStore } from "../../state/floorStore";
import { jumpTo } from "../../lib/floor/camera";
import { useRooms } from "../../lib/floor/rooms";
import { waypoints } from "../../lib/floor/waypoints";
import { ProjectBadge } from "../ProjectBadge";
import styles from "./FloorWaypoints.module.css";

export function FloorWaypoints() {
  const rooms = useRooms();
  const camera = useFloorStore((s) => s.camera);
  const viewport = useFloorStore((s) => s.viewport);
  // shortcut: computed from store state, so during an animated jump the markers run one transition ahead of the picture.
  const marks = waypoints(rooms, camera, viewport);
  return (
    <div className={styles.layer}>
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
            style={{ transform: `translate(${m.x}px, ${m.y}px)` }}
            aria-label={`Go to ${project.name}: ${label}`}
            onClick={() => jumpTo(m.projectId)}
          >
            <span className={styles.shift}>
              <span
                className={styles.arrow}
                aria-hidden
                style={{ transform: `rotate(${m.angle}rad)` }}
              />
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
