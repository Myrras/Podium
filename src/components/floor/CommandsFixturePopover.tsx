/** The command panel's popover: pick a command to run it in a new terminal. */
import { useEffect, useId, useRef, useState } from "react";
import type { KeyboardEvent } from "react";

import type { ProjectId } from "../../ipc/types";
import { useTerminalCommands } from "../../lib/terminalCommands";
import { runCommand } from "../../lib/terminals";
import { useFloorStore } from "../../state/floorStore";
import { useProjectStore } from "../../state/projectStore";
import { EditIcon } from "../icons";
import { followCamera } from "../../lib/floor/follow";
import { Popover, PopoverSlot } from "../Popover";
import { TerminalCommandsModal } from "../TerminalCommandsModal";
import sidebarStyles from "../Sidebar.module.css";
import styles from "./CommandsFixturePopover.module.css";

const keyOf = (c: { name: string; command: string }) =>
  `${c.name}\t${c.command}`;

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
  const [query, setQuery] = useState("");
  // The highlighted command by identity: recipes load in front of the saved
  // commands later, which would shift an index.
  const [activeKey, setActiveKey] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const listId = useId();
  const q = query.trim().toLowerCase();
  const shown = q
    ? commands.filter(
        (c) =>
          c.name.toLowerCase().includes(q) ||
          c.command.toLowerCase().includes(q),
      )
    : commands;
  const hasList = !!project && commands.length > 0;
  const rowId = (i: number) => `${listId}-${i}`;
  const active =
    activeKey === null ? -1 : shown.findIndex((c) => keyOf(c) === activeKey);
  // The highlighted command is gone: clear the highlight.
  if (activeKey !== null && active < 0) setActiveKey(null);

  // After Popover's own focus-in (a layout effect), so the field wins.
  useEffect(() => {
    if (hasList) inputRef.current?.focus({ preventScroll: true });
  }, [hasList]);

  if (!project) return null;

  const run = (i: number) => {
    const c = shown[i];
    if (!c) return;
    void runCommand(project, c.command, c.name);
    closePopover();
  };
  const move = (i: number) => {
    setActiveKey(keyOf(shown[i]));
    document.getElementById(rowId(i))?.scrollIntoView?.({ block: "nearest" });
  };
  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Escape" && query) {
      e.preventDefault();
      e.stopPropagation();
      setQuery("");
      setActiveKey(null);
    } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (shown.length === 0) return;
      const d = e.key === "ArrowDown" ? 1 : -1;
      move(
        active < 0 && d < 0
          ? shown.length - 1
          : (active + d + shown.length) % shown.length,
      );
    } else if (e.key === "Enter") {
      e.preventDefault();
      run(active >= 0 ? active : 0);
    }
  };

  return (
    <>
      <Popover
        follow={followCamera}
        anchor={anchor}
        title="Commands"
        width={320}
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
        {!hasList ? (
          <p className={styles.empty}>
            No commands yet. Add one with Edit commands.
          </p>
        ) : (
          <>
            <input
              ref={inputRef}
              type="search"
              className={styles.search}
              placeholder="Search commands"
              aria-label="Search commands"
              aria-controls={listId}
              aria-activedescendant={active >= 0 ? rowId(active) : undefined}
              value={query}
              onChange={(e) => {
                setQuery(e.target.value);
                setActiveKey(null);
              }}
              onKeyDown={onKeyDown}
            />
            {shown.length === 0 ? (
              <p className={styles.empty}>No commands match "{query}".</p>
            ) : (
              <ul id={listId} className={styles.list}>
                {shown.map(({ command, name, index }, i) => (
                  <li key={index === null ? command : `custom-${index}`}>
                    <button
                      type="button"
                      id={rowId(i)}
                      tabIndex={-1}
                      data-active={i === active || undefined}
                      className={`ghost ${styles.row}`}
                      title={`Run in a new terminal: ${name}${name !== command ? ` · ${command}` : ""}`}
                      onClick={() => run(i)}
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
          </>
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
