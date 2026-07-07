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
 */

import { useEffect, useRef, useState } from "react";

import type { ProcessInfo, ProjectId, ProjectInfo, TodoId } from "../ipc/types";
import { useLayoutStore } from "../state/layoutStore";
import { useProcessStore } from "../state/processStore";
import { useProjectStore } from "../state/projectStore";
import { NewAgentModal } from "./NewAgentModal";
import { ProcessRow } from "./ProcessRow";
import { TodoSubsection } from "./TodoSubsection";
import {
  AddIcon,
  AgentIcon,
  CaretIcon,
  CloseIcon,
  EditIcon,
  ErrorIcon,
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
}

/** One entity subsection (Agents / Processes / Terminals) in a group. */
function Subsection({ title, Icon, rows, empty, add }: SubsectionProps) {
  return (
    <div className={styles.subsection}>
      <div className={styles.sectionHeader}>
        <Icon className={styles.panelIcon} />
        <span className={styles.panelTitle}>{title}</span>
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

  const addProcess = useProcessStore((s) => s.addProcess);
  const startProcess = useProcessStore((s) => s.startProcess);

  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(project.name);
  const inputRef = useRef<HTMLInputElement>(null);

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
    const info = await addProcess(project.id, {
      name: `Terminal ${terminals.length + 1}`,
      kind: "terminal",
    });
    if (info) await startProcess(info.id);
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
      <div
        className={styles.projectHeader}
        role="button"
        tabIndex={0}
        aria-expanded={!isCollapsed}
        aria-label={`${isCollapsed ? "Expand" : "Collapse"} ${project.name}`}
        onClick={toggleGroup}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            toggleGroup();
          }
        }}
      >
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
        <span
          className={styles.caretWrap}
          data-open={!isCollapsed ? "true" : undefined}
        >
          <CaretIcon />
        </span>
        <span className={styles.projectBadge} aria-hidden>
          {project.iconInitials}
        </span>
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
        {!editing && (
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
        )}
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
            rows={terminals}
            empty="No terminals yet."
            add={{ label: "New terminal", onClick: () => void addTerminal() }}
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

export function Sidebar() {
  const [agentModal, setAgentModal] = useState<AgentModalTarget | null>(null);
  const sidebarWidth = useLayoutStore((s) => s.sidebarWidth);
  const openTodoInWorkArea = useLayoutStore((s) => s.openTodoInWorkArea);

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
    </aside>
  );
}
