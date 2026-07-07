/**
 * Per-project to-do lists, keyed by project id.
 *
 * Mutations apply eagerly from the command's return value; the backend's
 * `todo:changed` event (which also fires when an agent edits to-dos over
 * MCP) triggers a `refresh` that reconciles.
 */

import { create } from "zustand";

import {
  todoAdd,
  todoAddLink,
  todoComment,
  todoCommentRemove,
  todoCommentUpdate,
  todoList,
  todoListArchived,
  todoRemove,
  todoRemoveLink,
  todoSetArchived,
  todoSetDone,
  todoUnassign,
  todoUpdate,
  toIpcError,
} from "../ipc/commands";
import type {
  CommentId,
  LinkId,
  ProjectId,
  TodoId,
  TodoInfo,
} from "../ipc/types";
import { toastError } from "./toastStore";

interface TodoState {
  todosByProject: Record<ProjectId, TodoInfo[]>;
  /** Archived to-dos per project, loaded on demand (the Archive modal). */
  archivedByProject: Record<ProjectId, TodoInfo[]>;
  /** Re-pull one project's active list (initial load + `todo:changed`). */
  refresh: (projectId: ProjectId) => Promise<void>;
  /** Re-pull one project's archived list (opening the Archive modal). */
  refreshArchived: (projectId: ProjectId) => Promise<void>;
  /**
   * Archive or unarchive a to-do; updates both the active and archived caches
   * eagerly. Returns the updated snapshot (or `null`).
   */
  setTodoArchived: (
    projectId: ProjectId,
    todoId: TodoId,
    archived: boolean,
  ) => Promise<TodoInfo | null>;
  /** Returns whether the add succeeded (the input clears only then). */
  addTodo: (projectId: ProjectId, text: string) => Promise<boolean>;
  setTodoDone: (
    projectId: ProjectId,
    todoId: TodoId,
    done: boolean,
  ) => Promise<void>;
  /**
   * Revise a to-do's text and/or description; returns the updated snapshot
   * (or `null` on failure) so callers can reflect the saved state.
   */
  updateTodo: (
    projectId: ProjectId,
    todoId: TodoId,
    changes: { text?: string; description?: string },
  ) => Promise<TodoInfo | null>;
  /** Append a progress note (author defaults to "You"); returns the snapshot. */
  commentTodo: (
    projectId: ProjectId,
    todoId: TodoId,
    text: string,
  ) => Promise<TodoInfo | null>;
  /** Revise a comment's text; returns the updated snapshot (or `null`). */
  editComment: (
    projectId: ProjectId,
    todoId: TodoId,
    commentId: CommentId,
    text: string,
  ) => Promise<TodoInfo | null>;
  /** Remove a comment; returns the updated snapshot (or `null`). */
  removeComment: (
    projectId: ProjectId,
    todoId: TodoId,
    commentId: CommentId,
  ) => Promise<TodoInfo | null>;
  /** Pin an issue/PR link (blank label falls back to the url); returns the snapshot. */
  addLink: (
    projectId: ProjectId,
    todoId: TodoId,
    url: string,
    label?: string,
  ) => Promise<TodoInfo | null>;
  /** Remove a pinned link; returns the updated snapshot (or `null`). */
  removeLink: (
    projectId: ProjectId,
    todoId: TodoId,
    linkId: LinkId,
  ) => Promise<TodoInfo | null>;
  removeTodo: (projectId: ProjectId, todoId: TodoId) => Promise<void>;
  /**
   * Unassign a to-do from its agent (sends a best-effort cancel request to the
   * agent first). Applies the returned snapshot eagerly; the `todo:changed`
   * refresh reconciles.
   */
  unassignTodo: (projectId: ProjectId, todoId: TodoId) => Promise<void>;
  /** Event applier for `project:closed` — drops the cached list. */
  dropProject: (projectId: ProjectId) => void;
}

export const useTodoStore = create<TodoState>((set) => ({
  todosByProject: {},
  archivedByProject: {},

  refresh: async (projectId) => {
    try {
      const todos = await todoList(projectId);
      set((s) => ({
        todosByProject: { ...s.todosByProject, [projectId]: todos },
      }));
    } catch (e) {
      toastError("Failed to list to-dos", toIpcError(e).message);
    }
  },

  refreshArchived: async (projectId) => {
    try {
      const todos = await todoListArchived(projectId);
      set((s) => ({
        archivedByProject: { ...s.archivedByProject, [projectId]: todos },
      }));
    } catch (e) {
      toastError("Failed to list archived to-dos", toIpcError(e).message);
    }
  },

  setTodoArchived: async (projectId, todoId, archived) => {
    try {
      const info = await todoSetArchived(projectId, todoId, archived);
      set((s) => {
        const active = s.todosByProject[projectId] ?? [];
        const archivedList = s.archivedByProject[projectId] ?? [];
        return {
          todosByProject: {
            ...s.todosByProject,
            [projectId]: archived
              ? active.filter((t) => t.id !== todoId)
              : active.some((t) => t.id === todoId)
                ? active.map((t) => (t.id === todoId ? info : t))
                : [...active, info],
          },
          archivedByProject: {
            ...s.archivedByProject,
            [projectId]: archived
              ? [info, ...archivedList.filter((t) => t.id !== todoId)]
              : archivedList.filter((t) => t.id !== todoId),
          },
        };
      });
      return info;
    } catch (e) {
      toastError(
        archived ? "Could not archive to-do" : "Could not unarchive to-do",
        toIpcError(e).message,
      );
      return null;
    }
  },

  addTodo: async (projectId, text) => {
    try {
      const info = await todoAdd(projectId, text);
      // Insert eagerly; the `todo:changed` refresh reconciles (insertion is
      // idempotent by id).
      set((s) => {
        const current = s.todosByProject[projectId] ?? [];
        if (current.some((t) => t.id === info.id)) return s;
        return {
          todosByProject: {
            ...s.todosByProject,
            [projectId]: [...current, info],
          },
        };
      });
      return true;
    } catch (e) {
      toastError("Could not add to-do", toIpcError(e).message);
      return false;
    }
  },

  setTodoDone: async (projectId, todoId, done) => {
    try {
      const info = await todoSetDone(projectId, todoId, done);
      set((s) => ({
        todosByProject: {
          ...s.todosByProject,
          [projectId]: (s.todosByProject[projectId] ?? []).map((t) =>
            t.id === todoId ? info : t,
          ),
        },
      }));
    } catch (e) {
      toastError("Could not update to-do", toIpcError(e).message);
    }
  },

  updateTodo: async (projectId, todoId, changes) => {
    try {
      const info = await todoUpdate(projectId, todoId, changes);
      set((s) => ({
        todosByProject: {
          ...s.todosByProject,
          [projectId]: (s.todosByProject[projectId] ?? []).map((t) =>
            t.id === todoId ? info : t,
          ),
        },
      }));
      return info;
    } catch (e) {
      toastError("Could not update to-do", toIpcError(e).message);
      return null;
    }
  },

  commentTodo: async (projectId, todoId, text) => {
    try {
      const info = await todoComment(projectId, todoId, text);
      set((s) => ({
        todosByProject: {
          ...s.todosByProject,
          [projectId]: (s.todosByProject[projectId] ?? []).map((t) =>
            t.id === todoId ? info : t,
          ),
        },
      }));
      return info;
    } catch (e) {
      toastError("Could not add comment", toIpcError(e).message);
      return null;
    }
  },

  editComment: async (projectId, todoId, commentId, text) => {
    try {
      const info = await todoCommentUpdate(projectId, todoId, commentId, text);
      set((s) => ({
        todosByProject: {
          ...s.todosByProject,
          [projectId]: (s.todosByProject[projectId] ?? []).map((t) =>
            t.id === todoId ? info : t,
          ),
        },
      }));
      return info;
    } catch (e) {
      toastError("Could not edit comment", toIpcError(e).message);
      return null;
    }
  },

  removeComment: async (projectId, todoId, commentId) => {
    try {
      const info = await todoCommentRemove(projectId, todoId, commentId);
      set((s) => ({
        todosByProject: {
          ...s.todosByProject,
          [projectId]: (s.todosByProject[projectId] ?? []).map((t) =>
            t.id === todoId ? info : t,
          ),
        },
      }));
      return info;
    } catch (e) {
      toastError("Could not remove comment", toIpcError(e).message);
      return null;
    }
  },

  addLink: async (projectId, todoId, url, label) => {
    try {
      const info = await todoAddLink(projectId, todoId, url, label);
      set((s) => ({
        todosByProject: {
          ...s.todosByProject,
          [projectId]: (s.todosByProject[projectId] ?? []).map((t) =>
            t.id === todoId ? info : t,
          ),
        },
      }));
      return info;
    } catch (e) {
      toastError("Could not add link", toIpcError(e).message);
      return null;
    }
  },

  removeLink: async (projectId, todoId, linkId) => {
    try {
      const info = await todoRemoveLink(projectId, todoId, linkId);
      set((s) => ({
        todosByProject: {
          ...s.todosByProject,
          [projectId]: (s.todosByProject[projectId] ?? []).map((t) =>
            t.id === todoId ? info : t,
          ),
        },
      }));
      return info;
    } catch (e) {
      toastError("Could not remove link", toIpcError(e).message);
      return null;
    }
  },

  removeTodo: async (projectId, todoId) => {
    try {
      await todoRemove(projectId, todoId);
      // Deletion happens from the Archive modal, so drop the to-do from both
      // caches (it may live in either while the modal reconciles).
      set((s) => ({
        todosByProject: {
          ...s.todosByProject,
          [projectId]: (s.todosByProject[projectId] ?? []).filter(
            (t) => t.id !== todoId,
          ),
        },
        archivedByProject: {
          ...s.archivedByProject,
          [projectId]: (s.archivedByProject[projectId] ?? []).filter(
            (t) => t.id !== todoId,
          ),
        },
      }));
    } catch (e) {
      toastError("Could not remove to-do", toIpcError(e).message);
    }
  },

  unassignTodo: async (projectId, todoId) => {
    try {
      const info = await todoUnassign(projectId, todoId);
      set((s) => ({
        todosByProject: {
          ...s.todosByProject,
          [projectId]: (s.todosByProject[projectId] ?? []).map((t) =>
            t.id === todoId ? info : t,
          ),
        },
      }));
    } catch (e) {
      toastError("Could not unassign to-do", toIpcError(e).message);
    }
  },

  dropProject: (projectId) =>
    set((s) => {
      const next = { ...s.todosByProject };
      delete next[projectId];
      const nextArchived = { ...s.archivedByProject };
      delete nextArchived[projectId];
      return { todosByProject: next, archivedByProject: nextArchived };
    }),
}));
