/** One process in a sidebar section: status dot, name, hover actions. */

import { useEffect, useRef, useState } from "react";

import type { ProcessInfo } from "../ipc/types";
import { activityLabel, isActive } from "../lib/processStatus";
import { useAgentActivity } from "../lib/useAgentActivity";
import { useProcessStore } from "../state/processStore";
import { AgentScratchpadList } from "./AgentScratchpadList";
import { AgentTodoList } from "./AgentTodoList";
import { StatusDot } from "./StatusDot";
import { CloseIcon, EditIcon, RestartIcon, RunIcon, StopIcon } from "./icons";
import styles from "./ProcessRow.module.css";

export function ProcessRow({ process }: { process: ProcessInfo }) {
  const activeProcessId = useProcessStore((s) => s.activeProcessId);
  const setActiveProcess = useProcessStore((s) => s.setActiveProcess);
  const startProcess = useProcessStore((s) => s.startProcess);
  const stopProcess = useProcessStore((s) => s.stopProcess);
  const restartProcess = useProcessStore((s) => s.restartProcess);
  const removeProcess = useProcessStore((s) => s.removeProcess);
  const renameProcess = useProcessStore((s) => s.renameProcess);

  const active = isActive(process.status);
  const selected = activeProcessId === process.id;
  const isAgent = process.kind.kind === "agent";
  // Services come from `podium.yml`, so their name is owned by the config.
  const renamable = isAgent || process.kind.kind === "terminal";
  const running = process.status.state === "running";
  const activity = useAgentActivity(process.id);

  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(process.name);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (editing) inputRef.current?.select();
  }, [editing]);

  const startEditing = () => {
    setActiveProcess(process.id);
    setDraft(process.name);
    setEditing(true);
  };

  const commitRename = () => {
    setEditing(false);
    const next = draft.trim();
    if (next.length === 0 || next === process.name) return;
    void renameProcess(process.id, next);
  };

  const cancelRename = () => {
    setEditing(false);
    setDraft(process.name);
  };

  return (
    <>
      <div
        className={styles.row}
        data-selected={selected ? "true" : undefined}
        role="button"
        tabIndex={0}
        onClick={() => setActiveProcess(process.id)}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            setActiveProcess(process.id);
          }
        }}
      >
        <StatusDot
          status={process.status}
          activity={isAgent ? activity : undefined}
        />
        {editing ? (
          <input
            ref={inputRef}
            className={styles.nameInput}
            value={draft}
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
            className={styles.name}
            title={process.command}
            onDoubleClick={
              renamable
                ? (e) => {
                    e.stopPropagation();
                    startEditing();
                  }
                : undefined
            }
          >
            {process.name}
          </span>
        )}
        {isAgent && running && !editing && (
          <span className={styles.activity} data-activity={activity}>
            {activityLabel(activity)}
          </span>
        )}
        <span className={styles.actions} onClick={(e) => e.stopPropagation()}>
          {renamable && !editing && (
            <button
              type="button"
              className={styles.action}
              aria-label={`Rename ${process.name}`}
              title="Rename"
              onClick={startEditing}
            >
              <EditIcon size={12} />
            </button>
          )}
          {/* Agents get no start/stop/restart controls — an agent session is
              a one-shot conversation, not a restartable service. */}
          {!isAgent && (
            <>
              {active ? (
                <button
                  type="button"
                  className={styles.action}
                  aria-label={`Stop ${process.name}`}
                  title="Stop"
                  onClick={() => void stopProcess(process.id)}
                >
                  <StopIcon size={12} />
                </button>
              ) : (
                <button
                  type="button"
                  className={styles.action}
                  aria-label={`Start ${process.name}`}
                  title="Start"
                  onClick={() => void startProcess(process.id)}
                >
                  <RunIcon size={12} />
                </button>
              )}
              <button
                type="button"
                className={styles.action}
                aria-label={`Restart ${process.name}`}
                title="Restart"
                onClick={() => void restartProcess(process.id)}
              >
                <RestartIcon size={12} />
              </button>
            </>
          )}
          <button
            type="button"
            className={`${styles.action} ${styles.actionDanger}`}
            aria-label={`Remove ${process.name}`}
            title="Remove"
            onClick={() => void removeProcess(process.id)}
          >
            <CloseIcon size={12} />
          </button>
        </span>
      </div>
      {isAgent && (
        <AgentTodoList projectId={process.projectId} processId={process.id} />
      )}
      {isAgent && (
        <AgentScratchpadList
          projectId={process.projectId}
          processId={process.id}
        />
      )}
    </>
  );
}
