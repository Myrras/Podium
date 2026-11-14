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
import {
  addSelectedItems,
  clearSelectedItems,
  toggleSelectedItem,
  useSelectionStore,
} from "../lib/selection";
import { textPreview } from "../lib/textPreview";
import { useContextMenu } from "../lib/useContextMenu";
import { startAgentOnTodo } from "../lib/todoAgent";
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
import { todoMenu } from "../lib/floor/menus";
import { useSectionCollapsed } from "../lib/useSectionCollapsed";
import { PopoverSlot } from "./Popover";
import { SectionTitle } from "./SectionTitle";
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
  /** Popover list: owner edge, full-title tooltip, Start as the one action. */
  bare?: boolean;
  onContextMenu?: (e: React.MouseEvent) => void;
}

/** Whether a running agent owns the to-do. A stopped owner keeps its link
 * (hollow square) but no longer blocks spawning on it or selecting it. */
const ownedByLiveAgent = (todo: TodoInfo) =>
  todo.assignedAgent?.active === true;

/** One to-do: a checkbox, a click-to-open title, and hover actions. */
function TodoRow({
  todo,
  multiSelected,
  open,
  onToggleDone,
  onActivate,
  onSpawn,
  onArchive,
  bare,
  onContextMenu,
}: TodoRowProps) {
  const commentCount = todo.comments.length;
  const assigned = todo.assignedAgent;
  const ownerTitle = assigned
    ? `Owned by ${assigned.name}${assigned.active ? "" : " (stopped)"}`
    : undefined;

  return (
    <div
      className={styles.row}
      data-done={todo.done ? "true" : undefined}
      data-multiselect={multiSelected ? "true" : undefined}
      data-open={open ? "true" : undefined}
      data-assigned={assigned ? "true" : undefined}
      onContextMenu={onContextMenu}
      style={
        // A stopped owner may have no colour: the square falls back to grey.
        assigned
          ? ({
              "--session-color": assigned.color ?? "var(--text-faint)",
            } as React.CSSProperties)
          : undefined
      }
    >
      <div className={styles.rowMain}>
        <span
          className={styles.owner}
          data-owned={assigned ? "true" : undefined}
          data-stopped={assigned && !assigned.active ? "true" : undefined}
          role={ownerTitle ? "img" : undefined}
          aria-label={ownerTitle}
          title={ownerTitle}
        />
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
          title={
            bare
              ? todo.text
              : `Open "${textPreview(todo.text)}" — Cmd/Ctrl+click to select`
          }
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
        {/* While its agent runs, a to-do is owned by that one session — no
            spawning a second agent on it. */}
        {!ownedByLiveAgent(todo) && (
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
        {/* The popover keeps rows quiet: archive is in the right-click menu. */}
        {!bare && (
          <>
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
          </>
        )}
      </div>
    </div>
  );
}

interface TodoSubsectionProps {
  projectId: ProjectId;
  /** Project root: the key its collapse state is persisted under. */
  projectRoot: string;
  /** Popover use: no section header or collapse, always open. */
  bare?: boolean;
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
  projectRoot,
  bare = false,
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
  const menu = useContextMenu();
  const inputRef = useRef<HTMLInputElement>(null);
  // Typed but not yet added: closing the Floor popover unmounts the input
  // without a blur, so the cleanup adds it. Esc and a successful add clear it.
  const pendingText = useRef("");
  // The add in flight: the cleanup waits for it, so a close during a
  // successful add does not add the same text twice.
  const inFlightAdd = useRef<{ text: string; ok: Promise<boolean> } | null>(
    null,
  );
  useEffect(
    () => () => {
      const pending = pendingText.current.trim();
      pendingText.current = "";
      if (!pending) return;
      const add = () =>
        void useTodoStore.getState().addTodo(projectId, pending);
      const busy = inFlightAdd.current;
      if (!busy) return add();
      void busy.ok.then((ok) => {
        if (!ok || busy.text !== pending) add();
      });
    },
    [projectId],
  );

  // Multi-select lives in the shared selection store — the assistant
  // composer shows the same rows as context chips. Only the Shift+range
  // anchor stays local; nothing outside this list cares about it.
  const allSelected = useSelectionStore((s) => s.items);
  const selected = useMemo(
    () =>
      new Set(
        allSelected
          .filter((i) => i.kind === "todo" && i.projectId === projectId)
          .map((i) => i.id),
      ),
    [allSelected, projectId],
  );
  const anchorRef = useRef<TodoId | null>(null);

  // Initial pull; later changes arrive via the `todo:changed` refresh.
  useEffect(() => {
    void refresh(projectId);
  }, [projectId, refresh]);

  // Ids to spawn on, in list order (stable, matches what the user sees).
  // To-dos a running agent owns can't be part of a selection.
  const selectedIds = useMemo(
    () =>
      todos
        .filter((t) => selected.has(t.id) && !ownedByLiveAgent(t))
        .map((t) => t.id),
    [todos, selected],
  );

  // The to-do currently open in the work area, highlighted like a selected
  // agent/terminal row. A Floor popover is not the work area: no highlight.
  const openTodoId =
    !bare && openTodo?.projectId === projectId ? openTodo.todoId : null;

  useEffect(() => {
    if (adding) inputRef.current?.focus();
  }, [adding]);

  const submit = async () => {
    const trimmed = text.trim();
    if (!trimmed) {
      pendingText.current = "";
      setAdding(false);
      setText("");
      return;
    }
    const ok = addTodo(projectId, trimmed);
    const busy = { text: trimmed, ok };
    inFlightAdd.current = busy;
    const added = await ok;
    if (inFlightAdd.current === busy) inFlightAdd.current = null;
    if (!added) return;
    // Kept when edited meanwhile: that text is still unsaved.
    if (pendingText.current.trim() === trimmed) pendingText.current = "";
    setText("");
  };

  const cancel = () => {
    pendingText.current = "";
    setAdding(false);
    setText("");
  };

  const spawnOnTodo = (e: React.MouseEvent, todo: TodoInfo) =>
    startAgentOnTodo(projectId, todo, e, onPickAgent);

  // A row's title click: Cmd/Ctrl toggles selection, Shift extends a range
  // from the anchor, a plain click opens the to-do to read/edit it. A plain
  // click is a "view" gesture, not a selection one, so it leaves any existing
  // selection (and its anchor) intact — clearing is done via the selection
  // bar's clear button or by toggling rows off.
  const activateTodo = (e: React.MouseEvent, todoId: TodoId) => {
    // A to-do a running agent owns is that session's — no multi-select on
    // it; any click just opens it.
    const clicked = todos.find((t) => t.id === todoId);
    if (clicked && ownedByLiveAgent(clicked)) {
      onOpenTodo(projectId, todoId);
      return;
    }
    if (e.metaKey || e.ctrlKey) {
      toggleSelectedItem({ kind: "todo", projectId, id: todoId });
      anchorRef.current = todoId;
      return;
    }
    if (e.shiftKey && anchorRef.current) {
      const from = todos.findIndex((t) => t.id === anchorRef.current);
      const to = todos.findIndex((t) => t.id === todoId);
      if (from !== -1 && to !== -1) {
        const [lo, hi] = from <= to ? [from, to] : [to, from];
        addSelectedItems(
          todos
            // Skip owned rows — they can't join a selection.
            .slice(lo, hi + 1)
            .filter((t) => !ownedByLiveAgent(t))
            .map((t) => ({ kind: "todo" as const, projectId, id: t.id })),
        );
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
    clearSelectedItems("todo", projectId);
    anchorRef.current = null;
  };

  const [storedCollapsed, setCollapsed] = useSectionCollapsed(
    projectRoot,
    "todos",
  );
  const collapsed = bare ? false : storedCollapsed;

  const headerActions = (
    <>
      <button
        type="button"
        className={`${sidebarStyles.addBtn} ${bare ? "" : sidebarStyles.hoverAction}`}
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
        onClick={() => {
          // bare (a Floor popover) never touches the sidebar's saved collapse state.
          if (!bare) setCollapsed(false);
          setAdding(true);
        }}
      >
        <AddIcon size={13} />
      </button>
    </>
  );

  const addForm = adding && (
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
        onChange={(e) => {
          setText(e.target.value);
          pendingText.current = e.target.value;
        }}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.preventDefault();
            cancel();
          }
        }}
        onBlur={() => void submit()}
      />
    </form>
  );

  return (
    <div
      className={sidebarStyles.subsection}
      data-collapsed={collapsed ? "true" : undefined}
    >
      {bare ? (
        <PopoverSlot name="header">{headerActions}</PopoverSlot>
      ) : (
        <div className={sidebarStyles.sectionHeader}>
          <SectionTitle
            title="To-dos"
            Icon={TodoIcon}
            count={todos.length}
            collapsed={collapsed}
            onToggle={() => setCollapsed(!collapsed)}
          />
          {headerActions}
        </div>
      )}
      {/* The popover counts open to-dos: its placeholder shows at 0 open,
          above any done ones still waiting to be archived. */}
      {!collapsed &&
        !adding &&
        (bare ? todos.every((t) => t.done) : todos.length === 0) && (
          <div className={sidebarStyles.placeholder}>
            {bare ? "No open to-dos. Add one with +." : "No to-dos yet."}
          </div>
        )}
      {!collapsed && (todos.length > 0 || adding) && (
        <div className={`${styles.rows} ${bare ? styles.bare : ""}`}>
          {bare && addForm}
          {todos.map((todo) => (
            <TodoRow
              key={todo.id}
              todo={todo}
              bare={bare}
              multiSelected={selected.has(todo.id)}
              open={openTodoId === todo.id}
              onToggleDone={() =>
                void setTodoDone(projectId, todo.id, !todo.done)
              }
              onActivate={(e) => activateTodo(e, todo.id)}
              onSpawn={(e) => spawnOnTodo(e, todo)}
              onArchive={() => void setTodoArchived(projectId, todo.id, true)}
              onContextMenu={
                bare
                  ? (e) => menu.open(e, todoMenu(projectId, todo))
                  : undefined
              }
            />
          ))}
          {!bare && addForm}
        </div>
      )}
      {/* Stays visible while collapsed: the selection outlives the fold, so
          its "Start agent" / Clear actions must too. */}
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
              clearSelectedItems("todo", projectId);
              anchorRef.current = null;
            }}
          >
            <CloseIcon size={12} />
          </button>
        </div>
      )}
      {menu.element}
      <ArchiveModal
        open={archiveOpen}
        projectId={projectId}
        onClose={() => setArchiveOpen(false)}
      />
    </div>
  );
}
