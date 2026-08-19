/**
 * Collapsible "Commands" block inside a project's Terminals subsection.
 *
 * Lists quick commands that each launch a new terminal running them: the
 * public recipes scanned from the project root's justfile (shown as
 * `just <name>`, re-read on every expand so edits show up) plus the user's
 * own commands, added inline via the header "+" and persisted per project in
 * localStorage (terminalCommandsStore). Collapse state persists the same way
 * and defaults to collapsed.
 */

import { useEffect, useRef, useState } from "react";

import { projectJustRecipes } from "../ipc/commands";
import type { ProjectId } from "../ipc/types";
import { useTerminalCommandsStore } from "../state/terminalCommandsStore";
import { AddIcon, CaretIcon, CloseIcon, RunIcon } from "./icons";
import sidebarStyles from "./Sidebar.module.css";
import styles from "./TerminalCommands.module.css";

interface TerminalCommandsProps {
  projectId: ProjectId;
  /** Project root path — the persistence key for custom commands. */
  root: string;
  /** Launch a new terminal running `command`. */
  onRun: (command: string) => void;
}

export function TerminalCommands({
  projectId,
  root,
  onRun,
}: TerminalCommandsProps) {
  const expanded = useTerminalCommandsStore(
    (s) => s.expandedByRoot[root] ?? false,
  );
  const custom = useTerminalCommandsStore((s) => s.customByRoot[root]);
  const toggleExpanded = useTerminalCommandsStore((s) => s.toggleExpanded);
  const addCommand = useTerminalCommandsStore((s) => s.addCommand);
  const removeCommand = useTerminalCommandsStore((s) => s.removeCommand);

  const [recipes, setRecipes] = useState<string[]>([]);
  const [adding, setAdding] = useState(false);
  const [draft, setDraft] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  // Re-scan the justfile on every expand, so edits show up without a reload.
  useEffect(() => {
    if (!expanded) return;
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
  }, [expanded, projectId]);

  useEffect(() => {
    if (adding) inputRef.current?.focus();
  }, [adding]);

  const submit = () => {
    const trimmed = draft.trim();
    if (!trimmed) {
      setAdding(false);
      setDraft("");
      return;
    }
    addCommand(root, trimmed);
    setDraft("");
  };

  const commands: { command: string; custom: boolean }[] = [
    ...recipes.map((r) => ({ command: `just ${r}`, custom: false })),
    ...(custom ?? []).map((c) => ({ command: c, custom: true })),
  ];

  return (
    <div className={styles.commands}>
      <div className={styles.headerRow}>
        <button
          type="button"
          className={styles.header}
          aria-expanded={expanded}
          onClick={() => toggleExpanded(root)}
        >
          <span className={styles.caretWrap} data-open={expanded || undefined}>
            <CaretIcon size={12} />
          </span>
          <span className={styles.headerTitle}>Commands</span>
        </button>
        <button
          type="button"
          className={sidebarStyles.addBtn}
          aria-label="Add command"
          title="Add command"
          onClick={() => {
            if (!expanded) toggleExpanded(root);
            setAdding(true);
          }}
        >
          <AddIcon size={13} />
        </button>
      </div>
      {expanded && (
        <div className={styles.rows}>
          {commands.map(({ command, custom: isCustom }) => (
            <div key={command} className={styles.row}>
              <button
                type="button"
                className={styles.run}
                title={`Run "${command}" in a new terminal`}
                onClick={() => onRun(command)}
              >
                <RunIcon size={11} className={styles.runIcon} />
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
                  <CloseIcon size={11} />
                </button>
              )}
            </div>
          ))}
          {adding && (
            <form
              className={styles.addForm}
              onSubmit={(e) => {
                e.preventDefault();
                submit();
              }}
            >
              <input
                ref={inputRef}
                className={styles.addInput}
                value={draft}
                placeholder="pnpm dev"
                aria-label="New command"
                onChange={(e) => setDraft(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Escape") {
                    setAdding(false);
                    setDraft("");
                  }
                }}
                onBlur={submit}
              />
            </form>
          )}
          {commands.length === 0 && !adding && (
            <div className={styles.empty}>
              No justfile recipes found — add your own with +.
            </div>
          )}
        </div>
      )}
    </div>
  );
}
