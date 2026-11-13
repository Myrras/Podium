/** The command panel's popover: pick a command to run it in a new terminal. */
import { useState } from "react";

import type { ProjectId } from "../../ipc/types";
import { useTerminalCommands } from "../../lib/terminalCommands";
import { runCommand } from "../../lib/terminals";
import { useFloorStore } from "../../state/floorStore";
import { useProjectStore } from "../../state/projectStore";
import { EditIcon } from "../icons";
import { Popover, PopoverSlot } from "../Popover";
import { TerminalCommandsModal } from "../TerminalCommandsModal";
import sidebarStyles from "../Sidebar.module.css";
import styles from "./CommandsFixturePopover.module.css";

export function CommandsFixturePopover({
  projectId,
  anchor,
}: {
  projectId: ProjectId;
  itemId: string | null;
  anchor: HTMLElement | null;
}) {
  const project = useProjectStore((s) =>
    s.projects.find((p) => p.id === projectId),
  );
  const closePopover = useFloorStore((s) => s.closePopover);
  const [editing, setEditing] = useState(false);
  const { commands } = useTerminalCommands(
    { id: projectId, root: project?.root ?? "" },
    !!project,
  );
  if (!project) return null;

  return (
    <>
      <Popover
        anchor={anchor}
        title="Commands"
        width={380}
        flush
        onClose={closePopover}
      >
        <PopoverSlot name="header">
          <button
            type="button"
            className={sidebarStyles.addBtn}
            aria-label="Edit commands"
            title="Edit commands"
            onClick={() => setEditing(true)}
          >
            <EditIcon size={13} />
          </button>
        </PopoverSlot>
        {commands.length === 0 ? (
          <p className={styles.empty}>
            No commands yet. Add one with Edit commands.
          </p>
        ) : (
          <ul className={styles.list}>
            {commands.map(({ command, name, index }) => (
              <li key={index === null ? command : `custom-${index}`}>
                <button
                  type="button"
                  className={`ghost ${styles.row}`}
                  title={`Run "${command}" in a new terminal`}
                  onClick={() => {
                    void runCommand(project, command, name);
                    closePopover();
                  }}
                >
                  <span className={styles.name}>{name}</span>
                  {name !== command && (
                    <span className={styles.command}>{command}</span>
                  )}
                </button>
              </li>
            ))}
          </ul>
        )}
      </Popover>
      <TerminalCommandsModal
        open={editing}
        projectId={projectId}
        root={project.root}
        onClose={() => setEditing(false)}
        onRun={(command, name, cwd) =>
          void runCommand(project, command, name, cwd)
        }
      />
    </>
  );
}
