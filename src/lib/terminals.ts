import type { ProjectInfo, WorktreeInfo } from "../ipc/types";
import { useProcessStore } from "../state/processStore";
import { useProjectStore } from "../state/projectStore";
import { terminalShellFor } from "./terminalShell";

/**
 * Start a new terminal in a project, or in one of its worktrees. Acting on a
 * project activates it. Takes `Pick` so callers holding only an id and the
 * WSL flag need no full `ProjectInfo`.
 */
export async function newTerminal(
  project: Pick<ProjectInfo, "id" | "nativeInWslMode">,
  worktree?: WorktreeInfo,
): Promise<void> {
  useProjectStore.getState().setActiveProject(project.id);
  const { processes, addProcess, startProcess } = useProcessStore.getState();
  const shell = terminalShellFor(project.nativeInWslMode);
  const count = processes.filter(
    (p) => p.projectId === project.id && p.kind.kind === "terminal",
  ).length;
  const info = await addProcess(project.id, {
    name: worktree ? `Terminal (${worktree.name})` : `Terminal ${count + 1}`,
    kind: "terminal",
    ...(worktree ? { cwd: `.podium/worktrees/${worktree.name}` } : {}),
    ...(shell ? { command: shell } : {}),
  });
  if (info) await startProcess(info.id);
}
