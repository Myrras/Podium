import type { ProcessInfo } from "../../ipc/types";
import type { RoomModel } from "../../lib/floor/types";
import { useFloorStore } from "../../state/floorStore";
import { useScratchpadStore } from "../../state/scratchpadStore";
import { useTodoStore } from "../../state/todoStore";
import styles from "./RoomWall.module.css";

const NO_ITEMS: never[] = [];

function lightOf(p: ProcessInfo): "up" | "crashed" | "exited" | "off" {
  const s = p.status;
  if (s.state === "running") return "up";
  if (s.state === "exited") return s.crashed ? "crashed" : "exited";
  return "off";
}

const more = (n: number, max: number) =>
  n > max ? <em className={styles.more}>+{n - max}</em> : null;

export function RoomWall({ room }: { room: RoomModel }) {
  const { project, services } = room;
  const openPopover = useFloorStore((s) => s.openPopover);
  const open = useFloorStore((s) => s.popover);
  const todos = useTodoStore((s) => s.todosByProject[project.id] ?? NO_ITEMS);
  const pads = useScratchpadStore(
    (s) => s.scratchpadsByProject[project.id] ?? NO_ITEMS,
  );
  const openTodos = todos.filter((t) => !t.done);
  const lights = services.map(lightOf);
  const sorted = [...lights].sort(
    (a, b) => Number(b === "crashed") - Number(a === "crashed"),
  );
  const crashed = lights.includes("crashed");

  const fixture = (
    kind: "todos" | "scratchpads" | "services",
    title: string,
    n: number,
    art: React.ReactNode,
  ) => (
    <button
      type="button"
      className={styles.fx}
      data-fixture={`${project.id}:${kind}`}
      data-on={open?.projectId === project.id && open.kind === kind}
      aria-label={`${title} of ${project.name}, ${n}`}
      onClick={() => openPopover({ projectId: project.id, kind, itemId: null })}
    >
      {art}
      <span className={styles.label}>
        {title} <b>{n}</b>
      </span>
    </button>
  );

  return (
    <div className={styles.wall}>
      {project.sections.todos &&
        fixture(
          "todos",
          "To-dos",
          openTodos.length,
          <span className={styles.board}>
            {openTodos.slice(0, 8).map((t) => {
              const a = t.assignedAgent;
              return (
                <i
                  key={t.id}
                  data-note=""
                  data-hollow={!!a && !a.active}
                  style={
                    a?.color
                      ? ({ "--note": a.color } as React.CSSProperties)
                      : undefined
                  }
                />
              );
            })}
            {more(openTodos.length, 8)}
          </span>,
        )}
      {project.sections.scratchpads &&
        fixture(
          "scratchpads",
          "Scratchpads",
          pads.length,
          <span className={styles.books}>
            {pads.slice(0, 4).map((p) => (
              <i key={p.id} />
            ))}
            {more(pads.length, 4)}
          </span>,
        )}
      {services.length > 0 &&
        fixture(
          "services",
          "Services",
          services.length,
          <>
            <span className={styles.rack}>
              {sorted.slice(0, 3).map((l, i) => (
                <i key={i} data-light={l} />
              ))}
              {more(services.length, 3)}
            </span>
            {crashed && (
              <span className={styles.bang} aria-hidden="true">
                !
              </span>
            )}
          </>,
        )}
    </div>
  );
}
