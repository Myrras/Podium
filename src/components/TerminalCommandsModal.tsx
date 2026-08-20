/**
 * "Commands" modal for a project's Terminals subsection.
 *
 * Lists quick commands that each launch a new terminal running them: the
 * public recipes scanned from the project root's justfile (shown as
 * `just <name>`, re-read on every open so edits show up) plus the user's own
 * commands, added via the form at the bottom and persisted per project in
 * localStorage (terminalCommandsStore). Running a command closes the modal.
 */

import { useEffect, useState } from "react";

import { projectJustRecipes } from "../ipc/commands";
import type { ProjectId } from "../ipc/types";
import { useTerminalCommandsStore } from "../state/terminalCommandsStore";
import { Modal } from "./Modal";
import { CloseIcon, RunIcon } from "./icons";
import styles from "./TerminalCommandsModal.module.css";

interface TerminalCommandsModalProps {
  open: boolean;
  projectId: ProjectId;
  /** Project root path — the persistence key for custom commands. */
  root: string;
  onClose: () => void;
  /** Launch a new terminal running `command`. */
  onRun: (command: string) => void;
}

export function TerminalCommandsModal({
  open,
  projectId,
  root,
  onClose,
  onRun,
}: TerminalCommandsModalProps) {
  const custom = useTerminalCommandsStore((s) => s.customByRoot[root]);
  const addCommand = useTerminalCommandsStore((s) => s.addCommand);
  const removeCommand = useTerminalCommandsStore((s) => s.removeCommand);

  const [recipes, setRecipes] = useState<string[]>([]);
  const [draft, setDraft] = useState("");

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

  const submit = () => {
    const trimmed = draft.trim();
    if (!trimmed) return;
    addCommand(root, trimmed);
    setDraft("");
  };

  const run = (command: string) => {
    onRun(command);
    onClose();
  };

  const commands: { command: string; custom: boolean }[] = [
    ...recipes.map((r) => ({ command: `just ${r}`, custom: false })),
    ...(custom ?? []).map((c) => ({ command: c, custom: true })),
  ];

  return (
    <Modal open={open} title="Commands" onClose={onClose} width={520}>
      {commands.length === 0 ? (
        <p className={styles.empty}>
          No justfile recipes found — add your own below.
        </p>
      ) : (
        <ul className={styles.list}>
          {commands.map(({ command, custom: isCustom }) => (
            <li key={command} className={styles.item}>
              <button
                type="button"
                className={styles.run}
                title={`Run "${command}" in a new terminal`}
                onClick={() => run(command)}
              >
                <RunIcon size={12} className={styles.runIcon} />
                <span className={styles.command}>{command}</span>
              </button>
              {isCustom && (
                <button
                  type="button"
                  className={styles.remove}
                  aria-label={`Remove command "${command}"`}
                  title="Remove command"
                  onClick={() => removeCommand(root, command)}
                >
                  <CloseIcon size={12} />
                </button>
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
          className={styles.addInput}
          value={draft}
          placeholder="pnpm dev"
          aria-label="New command"
          onChange={(e) => setDraft(e.target.value)}
        />
        <button type="submit" className="primary" disabled={!draft.trim()}>
          Add
        </button>
      </form>
    </Modal>
  );
}
