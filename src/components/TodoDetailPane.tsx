/**
 * Work-area pane for an opened to-do: a header (title, done toggle, close)
 * over the editable description (what the to-do is about) and the comment
 * thread (progress notes from you and agents) with a composer. The
 * description and comments also change from agents over MCP, so the pane
 * reads the live to-do from the store by id and reflects `todo:changed`
 * refreshes while open.
 */

import { useEffect, useRef, useState } from "react";

import type { CommentId, ProjectId, TodoId, TodoInfo } from "../ipc/types";
import { useLayoutStore } from "../state/layoutStore";
import { useTodoStore } from "../state/todoStore";
import {
  CloseIcon,
  CommentIcon,
  DeleteIcon,
  EditIcon,
  LinkIcon,
  TodoIcon,
} from "./icons";
import { Markdown } from "./Markdown";
import styles from "./TodoDetailPane.module.css";

/** Compact local-time label for a comment (e.g. "Apr 3, 14:05"). */
function formatTime(iso: string): string {
  return new Date(iso).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

const NO_TODOS: TodoInfo[] = [];

export function TodoDetailPane({
  projectId,
  todoId,
}: {
  projectId: ProjectId;
  todoId: TodoId;
}) {
  const todo = useTodoStore((s) =>
    (s.todosByProject[projectId] ?? NO_TODOS).find((t) => t.id === todoId),
  );
  const updateTodo = useTodoStore((s) => s.updateTodo);
  const commentTodo = useTodoStore((s) => s.commentTodo);
  const editComment = useTodoStore((s) => s.editComment);
  const removeComment = useTodoStore((s) => s.removeComment);
  const removeLink = useTodoStore((s) => s.removeLink);
  const setTodoDone = useTodoStore((s) => s.setTodoDone);
  const clearOpenTodo = useLayoutStore((s) => s.clearOpenTodo);

  const [description, setDescription] = useState("");
  const [comment, setComment] = useState("");
  const [busy, setBusy] = useState(false);
  // The comment currently open for editing / awaiting delete confirmation
  // (at most one at a time), plus the in-progress edit text.
  const [editingId, setEditingId] = useState<CommentId | null>(null);
  const [editText, setEditText] = useState("");
  const [confirmDeleteId, setConfirmDeleteId] = useState<CommentId | null>(
    null,
  );
  const savedRef = useRef<string>("");

  // Seed the editable description whenever a different to-do opens, or the
  // stored description changes underneath us (agent edit).
  const stored = todo?.description ?? "";
  useEffect(() => {
    setDescription(stored);
    savedRef.current = stored;
  }, [todoId, stored]);

  useEffect(() => {
    setComment("");
    setEditingId(null);
    setEditText("");
    setConfirmDeleteId(null);
  }, [todoId]);

  // The open to-do vanished (removed here or by an agent): close the pane.
  useEffect(() => {
    if (todo === undefined) clearOpenTodo();
  }, [todo, clearOpenTodo]);

  if (todo === undefined) return null;

  const dirty = description !== savedRef.current;

  const saveDescription = async () => {
    if (!dirty || busy) return;
    setBusy(true);
    const info = await updateTodo(projectId, todoId, { description });
    setBusy(false);
    if (info) savedRef.current = info.description ?? "";
  };

  const addComment = async () => {
    const text = comment.trim();
    if (!text || busy) return;
    setBusy(true);
    const info = await commentTodo(projectId, todoId, text);
    setBusy(false);
    if (info) setComment("");
  };

  const startEdit = (id: CommentId, text: string) => {
    setConfirmDeleteId(null);
    setEditingId(id);
    setEditText(text);
  };

  const cancelEdit = () => {
    setEditingId(null);
    setEditText("");
  };

  const saveEdit = async () => {
    const text = editText.trim();
    if (!text || editingId === null || busy) return;
    setBusy(true);
    const info = await editComment(projectId, todoId, editingId, text);
    setBusy(false);
    if (info) cancelEdit();
  };

  const deleteComment = async (id: CommentId) => {
    if (busy) return;
    setBusy(true);
    await removeComment(projectId, todoId, id);
    setBusy(false);
    setConfirmDeleteId(null);
  };

  return (
    <div className={styles.pane}>
      <header className={styles.header}>
        <TodoIcon className={styles.kindIcon} />
        <input
          type="checkbox"
          className={styles.checkbox}
          checked={todo.done}
          aria-label={`Mark "${todo.text}" as ${todo.done ? "not done" : "done"}`}
          onChange={() => void setTodoDone(projectId, todoId, !todo.done)}
        />
        <span
          className={styles.name}
          data-done={todo.done ? "true" : undefined}
          title={todo.text}
        >
          {todo.text}
        </span>
        <button
          type="button"
          className={styles.closeBtn}
          aria-label="Close to-do"
          title="Close"
          onClick={clearOpenTodo}
        >
          <CloseIcon />
        </button>
      </header>

      <div className={styles.body}>
        {todo.links.length > 0 && (
          <ul className={styles.links}>
            {todo.links.map((link) => (
              <li key={link.id} className={styles.link}>
                <LinkIcon className={styles.linkIcon} />
                <a
                  href={link.url}
                  target="_blank"
                  rel="noreferrer"
                  className={styles.linkAnchor}
                  title={link.url}
                >
                  {link.label}
                </a>
                <button
                  type="button"
                  className={styles.iconBtn}
                  aria-label={`Remove link ${link.label}`}
                  title="Remove link"
                  onClick={() => void removeLink(projectId, todoId, link.id)}
                >
                  <CloseIcon size={12} />
                </button>
              </li>
            ))}
          </ul>
        )}

        <section className={styles.field}>
          <div className={styles.fieldHeader}>
            <label htmlFor="todo-pane-description">Description</label>
            {dirty && (
              <button
                type="button"
                className="primary"
                disabled={busy}
                onClick={() => void saveDescription()}
              >
                {busy ? "Saving…" : "Save"}
              </button>
            )}
          </div>
          <textarea
            id="todo-pane-description"
            className={styles.description}
            value={description}
            rows={10}
            placeholder="What is this to-do about?"
            onChange={(e) => setDescription(e.target.value)}
          />
        </section>

        <section className={styles.field}>
          <label>
            <CommentIcon size={13} /> Comments
          </label>
          {todo.comments.length > 0 ? (
            <ul className={styles.comments}>
              {todo.comments.map((c) => (
                <li key={c.id} className={styles.comment}>
                  <div className={styles.commentMeta}>
                    <span className={styles.commentAuthor}>{c.author}</span>
                    <span className={styles.commentTime}>
                      {formatTime(c.createdAt)}
                      {c.editedAt ? " · edited" : ""}
                    </span>
                    {editingId !== c.id && (
                      <div className={styles.commentActions}>
                        {confirmDeleteId === c.id ? (
                          <>
                            <span className={styles.confirmLabel}>Delete?</span>
                            <button
                              type="button"
                              className={styles.confirmDelete}
                              disabled={busy}
                              onClick={() => void deleteComment(c.id)}
                            >
                              Delete
                            </button>
                            <button
                              type="button"
                              className={styles.linkBtn}
                              onClick={() => setConfirmDeleteId(null)}
                            >
                              Cancel
                            </button>
                          </>
                        ) : (
                          <>
                            <button
                              type="button"
                              className={styles.iconBtn}
                              aria-label="Edit comment"
                              title="Edit"
                              onClick={() => startEdit(c.id, c.text)}
                            >
                              <EditIcon />
                            </button>
                            <button
                              type="button"
                              className={styles.iconBtn}
                              aria-label="Delete comment"
                              title="Delete"
                              onClick={() => setConfirmDeleteId(c.id)}
                            >
                              <DeleteIcon />
                            </button>
                          </>
                        )}
                      </div>
                    )}
                  </div>
                  {editingId === c.id ? (
                    <div className={styles.editComment}>
                      <textarea
                        className={styles.commentInput}
                        value={editText}
                        rows={3}
                        aria-label="Edit comment"
                        onChange={(e) => setEditText(e.target.value)}
                        onKeyDown={(e) => {
                          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                            e.preventDefault();
                            void saveEdit();
                          } else if (e.key === "Escape") {
                            cancelEdit();
                          }
                        }}
                      />
                      <div className={styles.editActions}>
                        <button
                          type="button"
                          className={styles.linkBtn}
                          onClick={cancelEdit}
                        >
                          Cancel
                        </button>
                        <button
                          type="button"
                          className="primary"
                          disabled={busy || !editText.trim()}
                          onClick={() => void saveEdit()}
                        >
                          Save
                        </button>
                      </div>
                    </div>
                  ) : (
                    <div className={styles.commentText}>
                      <Markdown>{c.text}</Markdown>
                    </div>
                  )}
                </li>
              ))}
            </ul>
          ) : (
            <p className={styles.empty}>
              No comments yet. Add one to track progress.
            </p>
          )}
          <form
            className={styles.addComment}
            onSubmit={(e) => {
              e.preventDefault();
              void addComment();
            }}
          >
            <textarea
              className={styles.commentInput}
              value={comment}
              rows={2}
              placeholder="Add a comment…"
              onChange={(e) => setComment(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                  e.preventDefault();
                  void addComment();
                }
              }}
            />
            <button
              type="submit"
              className="primary"
              disabled={busy || !comment.trim()}
            >
              Comment
            </button>
          </form>
        </section>
      </div>
    </div>
  );
}
