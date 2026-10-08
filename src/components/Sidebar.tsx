/**
 * Left sidebar — a floating island listing every workspace project.
 *
 * Each project renders as a collapsible group: a header row (badge, name,
 * config-error indicator, reload / remove actions) above the per-project
 * To-dos / Agents / Processes / Terminals subsections. To-dos lead the group
 * (the workflow starts by capturing a to-do, then spawning an agent for it);
 * To-dos, Agents and Terminals always show (their headers carry the add
 * buttons); Processes appears only when the project has service processes. A
 * pinned footer below the scrollable groups adds new projects. Group collapse
 * state persists via layoutStore.
 *
 * Secondary actions stay out of the way: a section header shows only its "+"
 * at rest and reveals its other actions on hover/focus, and the project
 * header's own actions (rename / reload / remove) are hidden the same way.
 */

import { useEffect, useRef, useState } from "react";

import type {
  ProcessInfo,
  ProjectId,
  ProjectInfo,
  ScratchpadId,
  TodoId,
  WorktreeInfo,
} from "../ipc/types";
import { projectOpenInWarp, toIpcError } from "../ipc/commands";
import { terminalShellFor } from "../lib/terminalShell";
import { useLayoutStore } from "../state/layoutStore";
import { useProcessStore } from "../state/processStore";
import { useProjectStore } from "../state/projectStore";
import { type ThemeMode, useThemeStore } from "../state/themeStore";
import { toastError } from "../state/toastStore";
import { NewAgentModal } from "./NewAgentModal";
import { ProcessRow } from "./ProcessRow";
import { ScratchpadAgentModal } from "./ScratchpadAgentModal";
import { ScratchpadSubsection } from "./ScratchpadSubsection";
import { TerminalCommandsModal } from "./TerminalCommandsModal";
import { TodoSubsection } from "./TodoSubsection";
import { WorktreeSubsection } from "./WorktreeSubsection";
import {
  AddIcon,
  AgentIcon,
  CaretIcon,
  CloseIcon,
  EditIcon,
  ErrorIcon,
  ExternalLinkIcon,
  FolderOpenIcon,
  GripIcon,
  RestartIcon,
  RunIcon,
  TerminalIcon,
} from "./icons";
import styles from "./Sidebar.module.css";

interface SubsectionProps {
  title: string;
  Icon: typeof AgentIcon;
  rows: ProcessInfo[];
  /** Hint shown when there are no rows (omit to render rows only). */
  empty?: string;
  /** Header add button; omitted for sections without a create action. */
  add?: { label: string; onClick: () => void };
  /** Extra header actions, rendered before the add button. */
  extra?: { label: string; Icon: typeof AgentIcon; onClick: () => void }[];
}

/** One entity subsection (Agents / Processes / Terminals) in a group. */
function Subsection({ title, Icon, rows, empty, add, extra }: SubsectionProps) {
  return (
    <div className={styles.subsection}>
      <div className={styles.sectionHeader}>
        <Icon className={styles.panelIcon} />
        <span className={styles.panelTitle}>{title}</span>
        {extra?.map((action) => (
          <button
            key={action.label}
            type="button"
            className={`${styles.addBtn} ${styles.hoverAction}`}
            aria-label={action.label}
            title={action.label}
            onClick={action.onClick}
          >
            <action.Icon size={13} />
          </button>
        ))}
        {add && (
          <button
            type="button"
            className={styles.addBtn}
            aria-label={add.label}
            title={add.label}
            onClick={add.onClick}
          >
            <AddIcon size={13} />
          </button>
        )}
      </div>
      {rows.length > 0 ? (
        <div className={styles.rows}>
          {rows.map((p) => (
            <ProcessRow key={p.id} process={p} />
          ))}
        </div>
      ) : empty ? (
        <div className={styles.placeholder}>{empty}</div>
      ) : null}
    </div>
  );
}

interface ProjectGroupProps {
  project: ProjectInfo;
  processes: ProcessInfo[];
  onNewAgent: (projectId: ProjectId) => void;
  onOpenTodo: (projectId: ProjectId, todoId: TodoId) => void;
  onPickAgent: (
    projectId: ProjectId,
    todoIds: TodoId[],
    initialName: string,
  ) => void;
  onOpenScratchpad: (projectId: ProjectId, scratchpadId: ScratchpadId) => void;
  /**
   * Open the agent picker (Scratchpad agent modal) pre-filled for these
   * scratchpad(s). Always used for spawning — scratchpads have no
   * direct-spawn shortcut, unlike to-dos.
   */
  onPickScratchpadAgent: (
    projectId: ProjectId,
    scratchpadIds: ScratchpadId[],
    initialName: string,
  ) => void;
  /** Drag-to-reorder wiring, owned by the Sidebar (tracks the drop target). */
  dragging: boolean;
  dropTarget: boolean;
  onDragStart: (id: ProjectId) => void;
  onDragOverProject: (id: ProjectId) => void;
  onDragEnd: () => void;
  onDrop: () => void;
}

/** One workspace project: collapsible header + its entity subsections. */
function ProjectGroup({
  project,
  processes,
  onNewAgent,
  onOpenTodo,
  onPickAgent,
  onOpenScratchpad,
  onPickScratchpadAgent,
  dragging,
  dropTarget,
  onDragStart,
  onDragOverProject,
  onDragEnd,
  onDrop,
}: ProjectGroupProps) {
  const isCollapsed = useLayoutStore(
    (s) => s.collapsedProjects[project.root] ?? false,
  );
  const toggleProjectCollapsed = useLayoutStore(
    (s) => s.toggleProjectCollapsed,
  );

  const setActiveProject = useProjectStore((s) => s.setActiveProject);
  const closeProject = useProjectStore((s) => s.closeProject);
  const reloadProjectConfig = useProjectStore((s) => s.reloadProjectConfig);
  const renameProject = useProjectStore((s) => s.renameProject);
  const setProjectColor = useProjectStore((s) => s.setProjectColor);

  const addProcess = useProcessStore((s) => s.addProcess);
  const startProcess = useProcessStore((s) => s.startProcess);

  const [editing, setEditing] = useState(false);
  // `undefined` = closed; `null` = open targeting the project root; a
  // `WorktreeInfo` = open targeting that worktree (from its row's Commands
  // action).
  const [commandsTarget, setCommandsTarget] = useState<
    WorktreeInfo | null | undefined
  >(undefined);
  const [draft, setDraft] = useState(project.name);
  const inputRef = useRef<HTMLInputElement>(null);

  // Badge colour: `colorDraft` follows the picker while it is open (the
  // `input` event) so the badge recolours instantly, and only the native
  // `change` event — the picker committing — is persisted. Persisting every
  // `input` meant a workspace.json write plus a full project refresh per
  // drag step.
  const [colorDraft, setColorDraft] = useState<string | null>(null);
  const colorInputRef = useRef<HTMLInputElement>(null);
  const saving = useRef(0);
  const badgeColor = colorDraft ?? project.color;
  const themeMode = useThemeStore((s) => s.mode);
  // An uncoloured badge follows the theme's accent, so the picker opens on
  // that rather than on one theme's hard-coded hex.
  const accentHex = themeAccentHex(themeMode);

  useEffect(() => {
    const el = colorInputRef.current;
    if (!el) return;
    const commit = () => {
      const next = el.value;
      if (next === project.color) {
        setColorDraft(null);
        return;
      }
      // Keep showing the draft until the store has the saved colour, so the
      // badge does not flash back to the old one in between.
      // Only clear it if the user has not moved on to a newer draft meanwhile.
      saving.current += 1;
      void setProjectColor(project.id, next).finally(() => {
        saving.current -= 1;
        setColorDraft((d) => (d === next ? null : d));
      });
    };
    // A picker dismissed without a `change` must not leave its draft on the badge.
    // (A blur during a pending save keeps the draft; the save clears it.)
    const discard = () => {
      if (!saving.current) setColorDraft(null);
    };
    el.addEventListener("change", commit);
    el.addEventListener("blur", discard);
    return () => {
      el.removeEventListener("change", commit);
      el.removeEventListener("blur", discard);
    };
  }, [project.id, project.color, setProjectColor]);

  const resetColor = () => {
    setColorDraft(null);
    if (project.color) void setProjectColor(project.id, null);
  };

  useEffect(() => {
    if (editing) inputRef.current?.select();
  }, [editing]);

  const startEditing = () => {
    setActiveProject(project.id);
    setDraft(project.name);
    setEditing(true);
  };

  const commitRename = () => {
    setEditing(false);
    const next = draft.trim();
    // Clear the override (null) when the name is blank or unchanged from the
    // config/folder name; otherwise persist the new name.
    if (next === project.name) return;
    void renameProject(project.id, next.length > 0 ? next : null);
  };

  const cancelRename = () => {
    setEditing(false);
    setDraft(project.name);
  };

  const agents = processes.filter((p) => p.kind.kind === "agent");
  const services = processes.filter((p) => p.kind.kind === "service");
  const terminals = processes.filter((p) => p.kind.kind === "terminal");

  const toggleGroup = () => {
    setActiveProject(project.id);
    toggleProjectCollapsed(project.root);
  };

  const addTerminal = async () => {
    setActiveProject(project.id);
    const shell = terminalShellFor(project.nativeInWslMode);
    const info = await addProcess(project.id, {
      name: `Terminal ${terminals.length + 1}`,
      kind: "terminal",
      ...(shell ? { command: shell } : {}),
    });
    if (info) await startProcess(info.id);
  };

  // Quick command from the Commands block: a new terminal running it, named
  // after the command's own name (a justfile recipe, or a user-set name).
  // `cwd` is set when the Commands modal was opened targeting a worktree
  // instead of the project root.
  const runCommand = async (command: string, name: string, cwd?: string) => {
    setActiveProject(project.id);
    const info = await addProcess(project.id, {
      name,
      kind: "terminal",
      command,
      ...(cwd ? { cwd } : {}),
    });
    if (info) await startProcess(info.id);
  };

  const openInWarp = () => {
    setActiveProject(project.id);
    projectOpenInWarp(project.id).catch((e: unknown) =>
      toastError("Could not open Warp", toIpcError(e).message),
    );
  };

  return (
    <div
      className={`${styles.projectGroup}${dragging ? ` ${styles.dragging}` : ""}${
        dropTarget ? ` ${styles.dropTarget}` : ""
      }`}
      onDragOver={(e) => {
        e.preventDefault();
        onDragOverProject(project.id);
      }}
      onDrop={(e) => {
        e.preventDefault();
        onDrop();
      }}
    >
      {/* The header row is a plain mouse target; the caret is the real,
          focusable toggle. Making the whole row role="button" nested the
          badge picker and the action buttons inside a button, which screen
          readers flatten to just its label, and its Enter/Space handler
          swallowed Space meant for the picker. */}
      <div className={styles.projectHeader} onClick={toggleGroup}>
        <span
          className={styles.dragHandle}
          role="img"
          aria-label={`Drag to reorder ${project.name}`}
          title="Drag to reorder"
          draggable
          onClick={(e) => e.stopPropagation()}
          onDragStart={(e) => {
            e.stopPropagation();
            e.dataTransfer.effectAllowed = "move";
            onDragStart(project.id);
          }}
          onDragEnd={onDragEnd}
        >
          <GripIcon size={13} />
        </span>
        <button
          type="button"
          className={styles.caretButton}
          aria-expanded={!isCollapsed}
          aria-label={`${isCollapsed ? "Expand" : "Collapse"} ${project.name}`}
          onClick={(e) => {
            e.stopPropagation();
            toggleGroup();
          }}
        >
          <span
            className={styles.caretWrap}
            data-open={!isCollapsed ? "true" : undefined}
          >
            <CaretIcon />
          </span>
        </button>
        <label
          className={styles.projectBadge}
          title={
            project.color
              ? "Change colour (right-click to reset)"
              : "Change colour"
          }
          style={
            badgeColor
              ? { background: badgeColor, color: readableOn(badgeColor) }
              : undefined
          }
          onClick={(e) => e.stopPropagation()}
          onContextMenu={(e) => {
            e.preventDefault();
            resetColor();
          }}
        >
          <span aria-hidden>{project.iconInitials}</span>
          <input
            ref={colorInputRef}
            type="color"
            className={styles.badgeColorInput}
            aria-label={`Change colour of ${project.name}`}
            aria-keyshortcuts={project.color ? "Delete" : undefined}
            value={badgeColor ?? accentHex}
            onChange={(e) => setColorDraft(e.target.value)}
            onKeyDown={(e) => {
              e.stopPropagation();
              if (e.key === "Delete" || e.key === "Backspace") {
                e.preventDefault();
                resetColor();
              }
            }}
          />
        </label>
        {/* The name and its hover actions share one slot: the actions
            overlay the end of the name instead of reserving their width,
            and stay inside the slot so they never cover the caret, the
            badge or the podium.yml error icon. */}
        <div className={styles.nameSlot}>
          {editing ? (
            <input
              ref={inputRef}
              className={styles.projectNameInput}
              value={draft}
              aria-label={`Rename ${project.name}`}
              placeholder={project.name}
              onClick={(e) => e.stopPropagation()}
              onChange={(e) => setDraft(e.target.value)}
              onBlur={commitRename}
              onKeyDown={(e) => {
                e.stopPropagation();
                if (e.key === "Enter") {
                  e.preventDefault();
                  commitRename();
                } else if (e.key === "Escape") {
                  e.preventDefault();
                  cancelRename();
                }
              }}
            />
          ) : (
            <span
              className={styles.projectName}
              title={project.root}
              onDoubleClick={(e) => {
                e.stopPropagation();
                startEditing();
              }}
            >
              {project.name}
            </span>
          )}
          {/* Hidden while renaming so they never cover the input. */}
          {!editing && (
            <div className={styles.projectActions}>
              <button
                type="button"
                className={styles.projectAction}
                aria-label={`Rename ${project.name}`}
                title="Rename"
                onClick={(e) => {
                  e.stopPropagation();
                  startEditing();
                }}
              >
                <EditIcon size={12} />
              </button>
              <button
                type="button"
                className={styles.projectAction}
                aria-label={`Reload podium.yml for ${project.name}`}
                title="Reload podium.yml"
                onClick={(e) => {
                  e.stopPropagation();
                  void reloadProjectConfig(project.id);
                }}
              >
                <RestartIcon size={12} />
              </button>
              <button
                type="button"
                className={styles.projectAction}
                aria-label={`Remove ${project.name} from sidebar`}
                title="Remove from sidebar"
                onClick={(e) => {
                  e.stopPropagation();
                  void closeProject(project.id);
                }}
              >
                <CloseIcon size={12} />
              </button>
            </div>
          )}
        </div>
        {project.configError && (
          <span
            className={styles.configError}
            role="img"
            aria-label="podium.yml error"
            title={project.configError}
          >
            <ErrorIcon size={13} />
          </span>
        )}
      </div>
      <div
        className={`${styles.groupBody}${isCollapsed ? ` ${styles.groupBodyCollapsed}` : ""}`}
      >
        <div className={styles.groupBodyInner}>
          <TodoSubsection
            projectId={project.id}
            onOpenTodo={onOpenTodo}
            onPickAgent={onPickAgent}
          />
          <Subsection
            title="Agents"
            Icon={AgentIcon}
            rows={agents}
            empty="No agents yet."
            add={{
              label: "New agent",
              onClick: () => {
                setActiveProject(project.id);
                onNewAgent(project.id);
              },
            }}
          />
          {services.length > 0 && (
            <Subsection title="Processes" Icon={RunIcon} rows={services} />
          )}
          <Subsection
            title="Terminals"
            Icon={TerminalIcon}
            rows={terminals.filter((p) => p.worktree == null)}
            empty="No terminals yet."
            add={{ label: "New terminal", onClick: () => void addTerminal() }}
            extra={[
              {
                label: "Open in Warp",
                Icon: ExternalLinkIcon,
                onClick: openInWarp,
              },
              {
                label: "Commands",
                Icon: RunIcon,
                onClick: () => {
                  setActiveProject(project.id);
                  setCommandsTarget(null);
                },
              },
            ]}
          />
          <WorktreeSubsection
            projectId={project.id}
            nativeInWslMode={project.nativeInWslMode}
            terminals={terminals}
            onOpenCommands={(wt) => {
              setActiveProject(project.id);
              setCommandsTarget(wt);
            }}
          />
          <TerminalCommandsModal
            open={commandsTarget !== undefined}
            projectId={project.id}
            root={project.root}
            worktree={commandsTarget ?? null}
            onClose={() => setCommandsTarget(undefined)}
            onRun={(command, name, cwd) => void runCommand(command, name, cwd)}
          />
          <ScratchpadSubsection
            projectId={project.id}
            onOpenScratchpad={onOpenScratchpad}
            onPickAgent={onPickScratchpadAgent}
          />
        </div>
      </div>
    </div>
  );
}

/** What the New agent modal is opened for: a plain new agent, or a to-do. */
interface AgentModalTarget {
  projectId: ProjectId;
  todoIds?: TodoId[];
  initialName?: string;
}

/** What the Scratchpad agent modal is opened for. */
interface ScratchpadAgentModalTarget {
  projectId: ProjectId;
  scratchpadIds: ScratchpadId[];
  initialName?: string;
}

export function Sidebar() {
  const [agentModal, setAgentModal] = useState<AgentModalTarget | null>(null);
  const [scratchpadAgentModal, setScratchpadAgentModal] =
    useState<ScratchpadAgentModalTarget | null>(null);
  const sidebarWidth = useLayoutStore((s) => s.sidebarWidth);
  const openTodoInWorkArea = useLayoutStore((s) => s.openTodoInWorkArea);
  const openScratchpadInWorkArea = useLayoutStore(
    (s) => s.openScratchpadInWorkArea,
  );

  const projects = useProjectStore((s) => s.projects);
  const openProjectDialog = useProjectStore((s) => s.openProjectDialog);
  const reorderProjects = useProjectStore((s) => s.reorderProjects);

  const processes = useProcessStore((s) => s.processes);

  // Drag-to-reorder state: the id being dragged and the id it's hovering over
  // (the drop lands the dragged project just before the hovered one).
  const [dragId, setDragId] = useState<ProjectId | null>(null);
  const [overId, setOverId] = useState<ProjectId | null>(null);

  const finishDrag = () => {
    setDragId(null);
    setOverId(null);
  };

  const handleDrop = () => {
    if (dragId && overId && dragId !== overId) {
      void reorderProjects(dragId, overId);
    }
    finishDrag();
  };

  return (
    <aside
      className={styles.sidebar}
      style={{ width: sidebarWidth, minWidth: sidebarWidth }}
    >
      <div className={styles.groups}>
        {projects.length > 0 ? (
          projects.map((project) => (
            <ProjectGroup
              key={project.id}
              project={project}
              processes={processes.filter((p) => p.projectId === project.id)}
              dragging={dragId === project.id}
              dropTarget={
                dragId !== null &&
                overId === project.id &&
                dragId !== project.id
              }
              onDragStart={setDragId}
              onDragOverProject={(id) => {
                if (dragId !== null) setOverId(id);
              }}
              onDragEnd={finishDrag}
              onDrop={handleDrop}
              onNewAgent={(projectId) => setAgentModal({ projectId })}
              onOpenTodo={openTodoInWorkArea}
              onPickAgent={(projectId, todoIds, initialName) =>
                setAgentModal({ projectId, todoIds, initialName })
              }
              onOpenScratchpad={openScratchpadInWorkArea}
              onPickScratchpadAgent={(projectId, scratchpadIds, initialName) =>
                setScratchpadAgentModal({
                  projectId,
                  scratchpadIds,
                  initialName,
                })
              }
            />
          ))
        ) : (
          <div className={styles.placeholder}>
            No projects yet. Add one to get started.
          </div>
        )}
      </div>

      <div className={styles.footer}>
        <button
          type="button"
          className={styles.addProjectBtn}
          onClick={() => void openProjectDialog()}
        >
          <FolderOpenIcon />
          <span>Add project…</span>
        </button>
      </div>

      <NewAgentModal
        open={agentModal !== null}
        projectId={agentModal?.projectId ?? null}
        todoIds={agentModal?.todoIds}
        initialName={agentModal?.initialName}
        onClose={() => setAgentModal(null)}
      />
      <ScratchpadAgentModal
        open={scratchpadAgentModal !== null}
        projectId={scratchpadAgentModal?.projectId ?? null}
        scratchpadIds={scratchpadAgentModal?.scratchpadIds ?? []}
        initialName={scratchpadAgentModal?.initialName}
        onClose={() => setScratchpadAgentModal(null)}
      />
    </aside>
  );
}

/** Resolved `--accent` per theme; the CSS lookup is not free. */
const accentByTheme = new Map<ThemeMode, string>();

/**
 * The active theme's `--accent` as `#rrggbb`, what a colour input takes.
 * Read from the CSS tokens (once per theme) rather than duplicated here.
 */
function themeAccentHex(mode: ThemeMode): string {
  const cached = accentByTheme.get(mode);
  if (cached) return cached;
  const accent = getComputedStyle(document.documentElement)
    .getPropertyValue("--accent")
    .trim()
    .toLowerCase();
  const hex = /^#[0-9a-f]{6}$/.test(accent) ? accent : "#000000";
  accentByTheme.set(mode, hex);
  return hex;
}

/** Black or white initials, whichever reads better on a `#rrggbb` badge. */
function readableOn(hex: string): string {
  const n = parseInt(hex.slice(1), 16);
  const luma = 0.299 * (n >> 16) + 0.587 * ((n >> 8) & 255) + 0.114 * (n & 255);
  return luma > 150 ? "#000" : "#fff";
}
