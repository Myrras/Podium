/**
 * A project's quick commands: the public recipes scanned from its root
 * justfile (re-scanned whenever `enabled` turns on) plus the user's own saved
 * commands. Shared by the Commands modal and the floor's Commands popover.
 */

import { useEffect, useState } from "react";

import { projectJustRecipes } from "../ipc/commands";
import type { ProjectInfo } from "../ipc/types";
import { useTerminalCommandsStore } from "../state/terminalCommandsStore";

export interface QuickCommand {
  command: string;
  /** Recipe name, the saved name, or the command line itself. */
  name: string;
  /** Index in the saved list; null for a justfile recipe (not editable). */
  index: number | null;
}

export function useTerminalCommands(
  project: Pick<ProjectInfo, "id" | "root">,
  enabled = true,
) {
  const { id, root } = project;
  const custom = useTerminalCommandsStore((s) => s.customByRoot[root]);
  const addCommand = useTerminalCommandsStore((s) => s.addCommand);
  const updateCommand = useTerminalCommandsStore((s) => s.updateCommand);
  const removeCommand = useTerminalCommandsStore((s) => s.removeCommand);
  const [recipes, setRecipes] = useState<string[]>([]);

  useEffect(() => {
    if (!enabled) return;
    let stale = false;
    projectJustRecipes(id)
      .then((r) => {
        if (!stale) setRecipes(r);
      })
      .catch(() => {
        if (!stale) setRecipes([]);
      });
    return () => {
      stale = true;
    };
  }, [enabled, id]);

  const commands: QuickCommand[] = [
    // The terminal is named after the recipe rather than `just <recipe>`.
    ...recipes.map((r) => ({ command: `just ${r}`, name: r, index: null })),
    ...(custom ?? []).map((c, index) => ({
      command: c.command,
      name: c.name ?? c.command,
      index,
    })),
  ];

  return {
    commands,
    add: (command: string, name?: string) => addCommand(root, command, name),
    update: (index: number, draft: { name: string; command: string }) =>
      updateCommand(root, index, draft),
    remove: (index: number) => removeCommand(root, index),
  };
}
