/**
 * "To-dos" subsection for one sidebar project group — the lead subsection,
 * since the workflow starts by capturing a to-do then spawning an agent for
 * it.
 *
 * Checkbox rows with hover-revealed spawn-agent and archive buttons, plus an
 * inline add input opened from the header "+" (Enter adds and keeps the input
 * open for quick successive entry; Escape or blurring while blank closes it).
 * Clicking a row's text opens the to-do detail dialog to read/edit its
 * description and read/add comments. The list is shared with agents over
 * MCP, so rows and comments can appear or change without any local
 * interaction (via the `todo:changed` refresh).
 *
 * Cmd/Ctrl+click a row's text toggles it into a selection (Shift+click extends
 * a range from the last-clicked row); with 2+ selected, a bar appears to hand
 * them all to a single agent as one combined task.
 */

import { useEffect, useMemo, useRef, useState } from "react";

import type { ProjectId, TodoId, TodoInfo } from "../ipc/types";
import { useLayoutStore } from "../state/layoutStore";
import { useProcessStore } from "../state/processStore";
import { useProjectStore } from "../state/projectStore";
import { useTodoStore } from "../state/todoStore";
import { ArchiveModal } from "./ArchiveModal";
import { CopyIdButton } from "./CopyIdButton";
import {
  AddIcon,
  AgentIcon,
  ArchiveIcon,
  CloseIcon,
  CommentIcon,
  TodoIcon,
} from "./icons";
import sidebarStyles from "./Sidebar.module.css";
import styles from "./TodoSubsection.module.css";

/** Stable empty list so the selector doesn't re-render on every store set. */
const NO_TODOS: TodoInfo[] = [];

interface TodoRowProps {
  todo: TodoInfo;
  /** Part of a Cmd/Ctrl+click multi-select — queued to hand to one agent. */
  multiSelected: boolean;
  /** Currently open in the work area (a plain-click "view" gesture). */
  open: boolean;
  onToggleDone: () => void;
  /** Plain click opens; Cmd/Ctrl or Shift click drives selection. */
  onActivate: (e: React.MouseEvent) => void;
  /** Plain click spawns the default agent; Cmd/Ctrl click opens the picker. */
  onSpawn: (e: React.MouseEvent) => void;
  onArchive: () => void;
}

/** One to-do: a checkbox, a click-to-open title, and hover actions. */
function TodoRow({
  todo,
  multiSelected,
  open,
  onToggleDone,
  onActivate,
  onSpawn,
  onArchive,
}: TodoRowProps) {
  const commentCount = todo.comments.length;
  const assigned = todo.assignedAgent;

  return (
    <div
      className={styles.row}
      data-done={todo.done ? "true" : undefined}
      data-multiselect={multiSelected ? "true" : undefined}
      data-open={open ? "true" : undefined}
      data-assigned={assigned ? "true" : undefined}
      style={
        assigned?.color
          ? ({ "--session-color": assigned.color } as React.CSSProperties)
          : undefined
      }
    >
      <div className={styles.rowMain}>
        <input
          type="checkbox"
          className={styles.checkbox}
          checked={todo.done}
          aria-label={`Mark "${todo.text}" as ${todo.done ? "not done" : "done"}`}
          onChange={onToggleDone}
        />
        <button
          type="button"
          className={styles.textToggle}
          title={`Open "${todo.text}" — Cmd/Ctrl+click to select`}
          onClick={onActivate}
        >
          <span className={styles.text}>{todo.text}</span>
          {commentCount > 0 && (
            <span
              className={styles.commentCount}
              title={`${commentCount} comment(s)`}
            >
              <CommentIcon size={11} />
              {commentCount}
            </span>
          )}
        </button>
        {/* Once assigned, a to-do is owned by one session — no spawning a
            second agent on it. */}
        {!assigned && (
          <button
            type="button"
            className={styles.action}
            aria-label={`Start an agent on "${todo.text}"`}
            title="Start an agent on this to-do (Cmd/Ctrl+click to pick the agent)"
            onClick={onSpawn}
          >
            <AgentIcon size={13} />
          </button>
        )}
        <button
          type="button"
          className={styles.action}
          aria-label={`Archive to-do "${todo.text}"`}
          title="Archive to-do"
          onClick={onArchive}
        >
          <ArchiveIcon size={13} />
        </button>
        <CopyIdButton id={todo.id} className={styles.action} />
      </div>
    </div>
  );
}

interface TodoSubsectionProps {
  projectId: ProjectId;
  /** Open the to-do detail dialog (hosted by the sidebar). */
  onOpenTodo: (projectId: ProjectId, todoId: TodoId) => void;
  /**
   * Open the agent picker (New agent modal) pre-filled for these to-dos, so the
   * user can choose which agent runs them. Used for Cmd/Ctrl+click on the spawn
   * button; a plain click spawns the default agent immediately.
   */
  onPickAgent: (
    projectId: ProjectId,
    todoIds: TodoId[],
    initialName: string,
  ) => void;
}

export function TodoSubsection({
  projectId,
  onOpenTodo,
  onPickAgent,
}: TodoSubsectionProps) {
  const todos = useTodoStore((s) => s.todosByProject[projectId] ?? NO_TODOS);
  const refresh = useTodoStore((s) => s.refresh);
  const addTodo = useTodoStore((s) => s.addTodo);
  const setTodoDone = useTodoStore((s) => s.setTodoDone);
  const setTodoArchived = useTodoStore((s) => s.setTodoArchived);
  const spawnAgent = useProcessStore((s) => s.spawnAgent);
  const setActiveProject = useProjectStore((s) => s.setActiveProject);
  const openTodo = useLayoutStore((s) => s.openTodo);

  const [adding, setAdding] = useState(false);
  const [text, setText] = useState("");
  const [archiveOpen, setArchiveOpen] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  // Multi-select state: the chosen ids and the anchor row for Shift+range.
  const [selected, setSelected] = useState<Set<TodoId>>(new Set());
  const anchorRef = useRef<TodoId | null>(null);

  // Initial pull; later changes arrive via the `todo:changed` refresh.
  useEffect(() => {
    void refresh(projectId);
  }, [projectId, refresh]);

  // Drop ids that no longer exist (a to-do was removed or completed away),
  // so the selection and its count never reference stale rows.
  useEffect(() => {
    setSelected((prev) => {
      if (prev.size === 0) return prev;
      const live = new Set(todos.map((t) => t.id));
      const next = new Set([...prev].filter((id) => live.has(id)));
      return next.size === prev.size ? prev : next;
    });
  }, [todos]);

  // Ids to spawn on, in list order (stable, matches what the user sees).
  // Assigned to-dos can't be part of a selection (they're owned already).
  const selectedIds = useMemo(
    () =>
      todos
        .filter((t) => selected.has(t.id) && !t.assignedAgent)
        .map((t) => t.id),
    [todos, selected],
  );

  // The to-do currently open in the work area, highlighted like a selected
  // agent/terminal row.
  const openTodoId = openTodo?.projectId === projectId ? openTodo.todoId : null;

  useEffect(() => {
    if (adding) inputRef.current?.focus();
  }, [adding]);

  const submit = async () => {
    const trimmed = text.trim();
    if (!trimmed) {
      setAdding(false);
      setText("");
      return;
    }
    if (await addTodo(projectId, trimmed)) setText("");
  };

  const cancel = () => {
    setAdding(false);
    setText("");
  };

  // Plain click spawns the default agent immediately; Cmd/Ctrl+click opens the
  // picker so the user can choose which agent runs the to-do.
  const spawnOnTodo = (e: React.MouseEvent, todo: TodoInfo) => {
    setActiveProject(projectId);
    if (e.metaKey || e.ctrlKey) {
      onPickAgent(projectId, [todo.id], todo.text);
      return;
    }
    void spawnAgent(projectId, { todoIds: [todo.id] });
  };

  // A row's title click: Cmd/Ctrl toggles selection, Shift extends a range
  // from the anchor, a plain click opens the to-do to read/edit it. A plain
  // click is a "view" gesture, not a selection one, so it leaves any existing
  // selection (and its anchor) intact — clearing is done via the selection
  // bar's clear button or by toggling rows off.
  const activateTodo = (e: React.MouseEvent, todoId: TodoId) => {
    // An assigned to-do is owned by a session — no multi-select on it; any
    // click just opens it.
    if (todos.find((t) => t.id === todoId)?.assignedAgent) {
      onOpenTodo(projectId, todoId);
      return;
    }
    if (e.metaKey || e.ctrlKey) {
      setSelected((prev) => {
        const next = new Set(prev);
        if (next.has(todoId)) next.delete(todoId);
        else next.add(todoId);
        return next;
      });
      anchorRef.current = todoId;
      return;
    }
    if (e.shiftKey && anchorRef.current) {
      const from = todos.findIndex((t) => t.id === anchorRef.current);
      const to = todos.findIndex((t) => t.id === todoId);
      if (from !== -1 && to !== -1) {
        const [lo, hi] = from <= to ? [from, to] : [to, from];
        setSelected((prev) => {
          const next = new Set(prev);
          // Skip assigned rows — they can't join a selection.
          for (let i = lo; i <= hi; i++)
            if (!todos[i].assignedAgent) next.add(todos[i].id);
          return next;
        });
        return;
      }
    }
    onOpenTodo(projectId, todoId);
  };

  const spawnOnSelected = (e: React.MouseEvent) => {
    if (selectedIds.length === 0) return;
    setActiveProject(projectId);
    if (e.metaKey || e.ctrlKey) {
      // A group of to-dos has no single sensible name — leave it blank so the
      // agent names the session itself once it has read them all.
      onPickAgent(projectId, selectedIds, "");
    } else {
      void spawnAgent(projectId, { todoIds: selectedIds });
    }
    setSelected(new Set());
    anchorRef.current = null;
  };

  return (
    <div className={sidebarStyles.subsection}>
      <div className={sidebarStyles.sectionHeader}>
        <TodoIcon className={sidebarStyles.panelIcon} />
        <span className={sidebarStyles.panelTitle}>To-dos</span>
        <button
          type="button"
          className={sidebarStyles.addBtn}
          aria-label="View archived to-dos"
          title="Archived to-dos"
          onClick={() => {
            setActiveProject(projectId);
            setArchiveOpen(true);
          }}
        >
          <ArchiveIcon size={13} />
        </button>
        <button
          type="button"
          className={sidebarStyles.addBtn}
          aria-label="New to-do"
          title="New to-do"
          onClick={() => setAdding(true)}
        >
          <AddIcon size={13} />
        </button>
      </div>
      {todos.length > 0 || adding ? (
        <div className={styles.rows}>
          {todos.map((todo) => (
            <TodoRow
              key={todo.id}
              todo={todo}
              multiSelected={selected.has(todo.id)}
              open={openTodoId === todo.id}
              onToggleDone={() =>
                void setTodoDone(projectId, todo.id, !todo.done)
              }
              onActivate={(e) => activateTodo(e, todo.id)}
              onSpawn={(e) => spawnOnTodo(e, todo)}
              onArchive={() => void setTodoArchived(projectId, todo.id, true)}
            />
          ))}
          {adding && (
            <form
              className={styles.addForm}
              onSubmit={(e) => {
                e.preventDefault();
                void submit();
              }}
            >
              <input
                ref={inputRef}
                className={styles.addInput}
                value={text}
                placeholder="What needs doing?"
                aria-label="New to-do text"
                onChange={(e) => setText(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Escape") cancel();
                }}
                onBlur={() => void submit()}
              />
            </form>
          )}
        </div>
      ) : (
        <div className={sidebarStyles.placeholder}>No to-dos yet.</div>
      )}
      {selectedIds.length >= 2 && (
        <div className={styles.selectionBar}>
          <button
            type="button"
            className={styles.selectionSpawn}
            title="Start one agent on all selected to-dos (Cmd/Ctrl+click to pick the agent)"
            onClick={spawnOnSelected}
          >
            <AgentIcon size={13} />
            Start agent on {selectedIds.length} to-dos
          </button>
          <button
            type="button"
            className={styles.selectionClear}
            aria-label="Clear selection"
            title="Clear selection"
            onClick={() => {
              setSelected(new Set());
              anchorRef.current = null;
            }}
          >
            <CloseIcon size={12} />
          </button>
        </div>
      )}
      <ArchiveModal
        open={archiveOpen}
        projectId={projectId}
        onClose={() => setArchiveOpen(false)}
      />
    </div>
  );
}
