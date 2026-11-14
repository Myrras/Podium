import type { ProcessInfo } from "../../ipc/types";
import { useProcessActivity } from "../../lib/useAgentActivity";
import { pillState, pillWord } from "../../lib/processStatus";
import { formatElapsed } from "../../lib/subagents";
import { deskState, useRooms } from "../../lib/floor/rooms";
import { useFloorStore } from "../../state/floorStore";
import { useProcessStore } from "../../state/processStore";
import { useScratchpadStore } from "../../state/scratchpadStore";
import { useTodoStore } from "../../state/todoStore";
import styles from "./FloorStatusBar.module.css";

const HINT =
  "Click a desk or a fixture to open it. Drag a nameplate to move a room. Drag the floor to pan.";

const STATE_LABEL = {
  working: "Working",
  needs: "Needs you",
  idle: "Idle",
  ended: "Ended",
  crashed: "Crashed",
} as const;

const plural = (n: number, one: string, many = `${one}s`) =>
  `${n} ${n === 1 ? one : many}`;

function AgentDetails({ process }: { process: ProcessInfo }) {
  const activity = useProcessActivity(process.id);
  const todo = useTodoStore((s) =>
    s.todosByProject[process.projectId]?.find(
      (t) => t.assignedAgent?.processId === process.id,
    ),
  );
  const kind = process.kind.kind;
  // Desk states are agent words; a terminal or service reads like its pill (Up, Running).
  const word =
    kind === "agent"
      ? STATE_LABEL[deskState(process, activity)]
      : pillWord(pillState(process.status, kind, activity), kind);
  const since =
    activity.sinceMs === null
      ? ""
      : ` for ${formatElapsed(Math.max(0, Date.now() - activity.sinceMs))}`;
  const parts = [
    `${word}${since}`,
    todo?.text,
    process.worktree ? `worktree ${process.worktree}` : null,
    activity.subagents > 0 ? plural(activity.subagents, "sub-agent") : null,
    activity.shells > 0 ? plural(activity.shells, "background shell") : null,
  ].filter(Boolean);
  return <>{parts.join(" · ")}</>;
}

function RoomCounts({ projectId }: { projectId: string }) {
  const room = useRooms().find((r) => r.project.id === projectId);
  const openTodos = useTodoStore(
    (s) => s.todosByProject[projectId]?.filter((t) => !t.done).length ?? 0,
  );
  const scratchpads = useScratchpadStore(
    (s) => s.scratchpadsByProject[projectId]?.length ?? 0,
  );
  if (!room) return <>{HINT}</>;
  return (
    <>
      {[
        plural(room.agents.length, "agent"),
        plural(openTodos, "open to-do"),
        plural(scratchpads, "scratchpad"),
        plural(room.services.length, "service"),
      ].join(" · ")}
    </>
  );
}

export function FloorStatusBar() {
  const hover = useFloorStore((s) => s.hoverDesk);
  const focus = useFloorStore((s) => s.focus);
  const active = useProcessStore((s) => s.activeProcessId);
  const room = focus && focus !== "all" ? focus : null;
  // Hovered desk, else the pinned desk unless one room is in focus and the desk is elsewhere.
  const process = useProcessStore((s) => {
    const find = (id: string | null) =>
      id ? s.processes.find((p) => p.id === id) : undefined;
    const pinned = find(active);
    return (
      find(hover) ?? (room && pinned?.projectId !== room ? undefined : pinned)
    );
  });
  return (
    <div className={styles.bar} role="status">
      {process ? (
        <AgentDetails process={process} />
      ) : room ? (
        <RoomCounts projectId={room} />
      ) : (
        HINT
      )}
    </div>
  );
}
