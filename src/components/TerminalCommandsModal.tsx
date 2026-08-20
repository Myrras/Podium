/**
 * "Commands" modal for a project's Terminals subsection, or for one
 * Podium-managed worktree's row (`worktree` prop set).
 *
 * Lists quick commands that each launch a new terminal running them: the
 * public recipes scanned from the project root's justfile (shown as
 * `just <name>`, re-read on every open so edits show up) plus the user's own
 * commands, added and edited through the form at the bottom and persisted per
 * project in localStorage (terminalCommandsStore). Each own command can carry
 * a name — the launched terminal inherits it instead of the command line.
 * Running a command closes the modal.
 *
 * When opened with a `worktree`, the chosen command runs there instead of
 * the project root (its cwd is passed through to `onRun`), and the title
 * names the worktree so it's clear which target is in play.
 */

import { useEffect, useState } from "react";

import { projectJustRecipes } from "../ipc/commands";
import type { ProjectId, WorktreeInfo } from "../ipc/types";
import { useTerminalCommandsStore } from "../state/terminalCommandsStore";
import { Modal } from "./Modal";
import { CloseIcon, EditIcon, RunIcon } from "./icons";
import styles from "./TerminalCommandsModal.module.css";

interface TerminalCommandsModalProps {
  open: boolean;
  projectId: ProjectId;
  /** Project root path — the persistence key for custom commands. */
  root: string;
  /** Run commands in this worktree instead of the project root; `null`/omitted for the root. */
  worktree?: WorktreeInfo | null;
  onClose: () => void;
  /** Launch a new terminal running `command`, named `name`, in `cwd` (relative
   * to the project root; `undefined` for the project root itself). */
  onRun: (command: string, name: string, cwd?: string) => void;
}

export function TerminalCommandsModal({
  open,
  projectId,
  root,
  worktree,
  onClose,
  onRun,
}: TerminalCommandsModalProps) {
  const custom = useTerminalCommandsStore((s) => s.customByRoot[root]);
  const addCommand = useTerminalCommandsStore((s) => s.addCommand);
  const updateCommand = useTerminalCommandsStore((s) => s.updateCommand);
  const removeCommand = useTerminalCommandsStore((s) => s.removeCommand);

  const [recipes, setRecipes] = useState<string[]>([]);
  const [draft, setDraft] = useState({ name: "", command: "" });
  /** Index in the custom list being edited, or null when adding a new one. */
  const [editing, setEditing] = useState<number | null>(null);

  // Re-scan the justfile on every open, so edits show up without a reload.
  useEffect(() => {
    if (!open) return;
    let stale = false;
    projectJustRecipes(projectId)
      .then((r) => {
        if (!stale) setRecipes(r);
      })
      .catch(() => {
        if (!stale) setRecipes([]);
      });
    return () => {
      stale = true;
    };
  }, [open, projectId]);

  // A closed modal keeps no half-finished edit around for the next open.
  useEffect(() => {
    if (!open) {
      setDraft({ name: "", command: "" });
      setEditing(null);
    }
  }, [open]);

  const submit = () => {
    if (!draft.command.trim()) return;
    if (editing === null) addCommand(root, draft.command, draft.name);
    else updateCommand(root, editing, draft);
    setDraft({ name: "", command: "" });
    setEditing(null);
  };

  const run = (command: string, name: string) => {
    if (worktree) onRun(command, name, `.podium/worktrees/${worktree.name}`);
    else onRun(command, name);
    onClose();
  };

  const commands = [
    // Recipes are derived from the justfile, so they are not editable here;
    // the terminal is named after the recipe rather than `just <recipe>`.
    ...recipes.map((r) => ({
      command: `just ${r}`,
      name: r,
      index: null as number | null,
    })),
    ...(custom ?? []).map((c, index) => ({
      command: c.command,
      name: c.name ?? c.command,
      index,
    })),
  ];

  return (
    <Modal
      open={open}
      title={worktree ? `Commands — ${worktree.name}` : "Commands"}
      onClose={onClose}
      width={520}
    >
      {commands.length === 0 ? (
        <p className={styles.empty}>
          No justfile recipes found — add your own below.
        </p>
      ) : (
        <ul className={styles.list}>
          {commands.map(({ command, name, index }) => (
            <li
              key={index === null ? command : `custom-${index}`}
              className={styles.item}
            >
              <button
                type="button"
                className={styles.run}
                title={`Run "${command}" in a new terminal`}
                onClick={() => run(command, name)}
              >
                <RunIcon size={12} className={styles.runIcon} />
                <span className={styles.label}>
                  <span className={styles.name}>{name}</span>
                  {name !== command && (
                    <span className={styles.command}>{command}</span>
                  )}
                </span>
              </button>
              {index !== null && (
                <>
                  <button
                    type="button"
                    className={styles.iconButton}
                    aria-label={`Edit command "${name}"`}
                    title="Edit command"
                    onClick={() => {
                      setEditing(index);
                      setDraft({ name: name === command ? "" : name, command });
                    }}
                  >
                    <EditIcon size={12} />
                  </button>
                  <button
                    type="button"
                    className={`${styles.iconButton} ${styles.remove}`}
                    aria-label={`Remove command "${name}"`}
                    title="Remove command"
                    onClick={() => {
                      removeCommand(root, index);
                      if (editing === index) {
                        setEditing(null);
                        setDraft({ name: "", command: "" });
                      }
                    }}
                  >
                    <CloseIcon size={12} />
                  </button>
                </>
              )}
            </li>
          ))}
        </ul>
      )}
      <form
        className={styles.addForm}
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
      >
        <input
          className={styles.nameInput}
          value={draft.name}
          placeholder="Name (optional)"
          aria-label="Command name"
          onChange={(e) => setDraft({ ...draft, name: e.target.value })}
        />
        <input
          className={styles.addInput}
          value={draft.command}
          placeholder="pnpm dev"
          aria-label={editing === null ? "New command" : "Command"}
          onChange={(e) => setDraft({ ...draft, command: e.target.value })}
        />
        <button
          type="submit"
          className="primary"
          disabled={!draft.command.trim()}
        >
          {editing === null ? "Add" : "Save"}
        </button>
        {editing !== null && (
          <button
            type="button"
            onClick={() => {
              setEditing(null);
              setDraft({ name: "", command: "" });
            }}
          >
            Cancel
          </button>
        )}
      </form>
    </Modal>
  );
}
