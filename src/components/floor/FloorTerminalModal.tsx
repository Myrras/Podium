/**
 * The terminal of a desk (or an assistant sub-agent) in an overlay over the
 * floor. Not `Modal`: Esc must reach the terminal (Claude Code uses it), and
 * the card may not scale, which would break xterm's mouse mapping.
 */

import { useEffect, useRef, useState } from "react";
import type { KeyboardEvent, ReactNode } from "react";
import { createPortal } from "react-dom";

import type { ProcessInfo } from "../../ipc/types";
import { isActive, pillState } from "../../lib/processStatus";
import { subagentProcess } from "../../lib/subagents";
import {
  UNTRACKED_ACTIVITY,
  useAgentActivityStore,
} from "../../state/agentActivityStore";
import { useAssistantStore } from "../../state/assistantStore";
import { useFloorStore } from "../../state/floorStore";
import { useLayoutStore } from "../../state/layoutStore";
import { useProcessStore } from "../../state/processStore";
import {
  closeConfirm,
  closeProcess,
  openProcess,
  stopConfirm,
} from "../../lib/floor/menus";
import {
  CircleStopIcon,
  CloseIcon,
  DeleteIcon,
  NextIcon,
  RestartIcon,
  RunIcon,
} from "../icons";
import { TerminalPane } from "../TerminalPane";
import styles from "./FloorTerminalModal.module.css";

const FOCUSABLE =
  'button:not([disabled]), input, textarea, [href], [tabindex]:not([tabindex="-1"])';

// Tab stays inside the element it is bound to. A focused terminal never gets
// here: xterm cancels Tab (and Esc) itself and sends it to the PTY.
function trapTab(e: KeyboardEvent<HTMLDivElement>) {
  if (e.key !== "Tab" || e.defaultPrevented) return;
  const items = Array.from(
    e.currentTarget.querySelectorAll<HTMLElement>(FOCUSABLE),
  );
  if (items.length === 0) return;
  const first = items[0];
  const last = items[items.length - 1];
  const at = document.activeElement;
  if (e.shiftKey && (at === first || !e.currentTarget.contains(at))) {
    e.preventDefault();
    last.focus();
  } else if (!e.shiftKey && (at === last || !e.currentTarget.contains(at))) {
    e.preventDefault();
    first.focus();
  }
}

export function FloorTerminalModal() {
  const terminalId = useFloorStore((s) => s.terminalId);
  const closeTerminal = useFloorStore((s) => s.closeTerminal);
  const openSubagentId = useLayoutStore((s) => s.openSubagent);
  const clearOpenSubagent = useLayoutStore((s) => s.clearOpenSubagent);
  const subagent = useAssistantStore(
    (s) => s.subagents.find((a) => a.id === openSubagentId) ?? null,
  );
  const process = useProcessStore(
    (s) => s.processes.find((p) => p.id === terminalId) ?? null,
  );
  const renameProcess = useProcessStore((s) => s.renameProcess);
  const startProcess = useProcessStore((s) => s.startProcess);
  const stopProcess = useProcessStore((s) => s.stopProcess);
  const restartProcess = useProcessStore((s) => s.restartProcess);
  const [confirm, setConfirm] = useState<Confirm | null>(null);
  // A question about one process never carries over to the next one shown.
  const [askedFor, setAskedFor] = useState({ terminalId, openSubagentId });
  if (
    askedFor.terminalId !== terminalId ||
    askedFor.openSubagentId !== openSubagentId
  ) {
    setAskedFor({ terminalId, openSubagentId });
    setConfirm(null);
  }

  // The process was removed elsewhere (or the sub-agent expired): close.
  useEffect(() => {
    if (openSubagentId !== null && subagent === null) clearOpenSubagent();
  }, [openSubagentId, subagent, clearOpenSubagent]);
  useEffect(() => {
    if (terminalId !== null && process === null) closeTerminal();
  }, [terminalId, process, closeTerminal]);

  const shown = subagent ? subagentProcess(subagent) : process;
  if (!shown) return null;
  const close = subagent ? clearOpenSubagent : closeTerminal;

  const rename = (name: string | null) => {
    // The store cannot clear a name: a blank one keeps the current name.
    if (name !== null) void renameProcess(shown.id, name);
  };
  const actions = subagent ? null : processActions(shown);

  return createPortal(
    <div
      className={styles.overlay}
      role="presentation"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) close();
      }}
    >
      <div
        className={styles.card}
        role="dialog"
        aria-modal="true"
        aria-label={shown.name}
        onKeyDown={trapTab}
      >
        <TerminalPane
          process={shown}
          leading={actions?.lifecycle}
          trailing={
            <>
              {actions?.process && (
                <>
                  <span className={styles.group}>{actions.process}</span>
                  <span className={styles.divider} aria-hidden="true" />
                </>
              )}
              {!subagent && <NextNeedsYou currentId={shown.id} />}
              <IconButton
                className={styles.hide}
                label={`Hide ${shown.name}`}
                title="Hide (keeps running)"
                onClick={close}
              >
                <CloseIcon />
              </IconButton>
            </>
          }
          onRename={subagent ? undefined : rename}
          hideActions={!subagent}
        />
        {confirm && (
          <ConfirmDialog {...confirm} onCancel={() => setConfirm(null)} />
        )}
      </div>
    </div>,
    document.body,
  );

  /**
   * Stop while active, else Resume (agents) or Start; Restart for running
   * terminals and services; Close for agents and terminals in any state.
   */
  function processActions(p: ProcessInfo): {
    lifecycle: ReactNode;
    process: ReactNode;
  } {
    const kind = p.kind.kind;
    const active = isActive(p.status);
    const start = kind === "agent" ? "Resume" : "Start";
    const restart = active && kind !== "agent";
    const closable = kind !== "service";
    return {
      lifecycle: active ? (
        <IconButton
          label={`Stop ${p.name}`}
          title="Stop"
          onClick={() =>
            setConfirm({
              ...stopConfirm(p),
              run: () => void stopProcess(p.id),
            })
          }
        >
          <CircleStopIcon />
        </IconButton>
      ) : (
        <IconButton
          label={`${start} ${p.name}`}
          title={start}
          onClick={() => void startProcess(p.id)}
        >
          <RunIcon />
        </IconButton>
      ),
      process: (restart || closable) && (
        <>
          {restart && (
            <IconButton
              label={`Restart ${p.name}`}
              title="Restart"
              onClick={() => void restartProcess(p.id)}
            >
              <RestartIcon />
            </IconButton>
          )}
          {closable && (
            <button
              type="button"
              className={styles.close}
              title={
                kind === "agent"
                  ? "Stop and remove the desk"
                  : "Stop and remove, with its output"
              }
              onClick={() =>
                setConfirm({ ...closeConfirm(p), run: () => closeProcess(p) })
              }
            >
              <DeleteIcon />
              {kind === "agent" ? "Close agent" : "Close terminal"}
            </button>
          )}
        </>
      ),
    };
  }
}

type Confirm = { title: string; body: string; action: string; run: () => void };

function IconButton({
  label,
  title,
  onClick,
  className = "",
  children,
}: {
  label: string;
  title: string;
  onClick: () => void;
  className?: string;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      className={`${styles.iconBtn} ${className}`}
      aria-label={label}
      title={title}
      onClick={onClick}
    >
      {children}
    </button>
  );
}

/**
 * Asks before a destructive action, inside the card. Not `Modal`: that one
 * closes on Esc through a document listener. Esc here only cancels.
 */
function ConfirmDialog({
  title,
  body,
  action,
  run,
  onCancel,
}: Confirm & { onCancel: () => void }) {
  const cancelRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    const opener = document.activeElement as HTMLElement | null;
    cancelRef.current?.focus();
    return () => opener?.focus();
  }, []);

  return (
    <div className={styles.scrim}>
      <div
        className={styles.confirm}
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="floor-confirm-title"
        aria-describedby="floor-confirm-body"
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.preventDefault();
            e.stopPropagation();
            onCancel();
          } else if (e.key === "Tab") {
            trapTab(e);
            // The card's own trap must not move focus out of the dialog.
            e.stopPropagation();
          }
        }}
      >
        <h2 id="floor-confirm-title" className={styles.confirmTitle}>
          {title}
        </h2>
        <p id="floor-confirm-body" className={styles.confirmBody}>
          {body}
        </p>
        <div className={styles.confirmActions}>
          <button ref={cancelRef} type="button" onClick={onCancel}>
            Cancel
          </button>
          <button
            type="button"
            className="danger"
            onClick={() => {
              onCancel();
              run();
            }}
          >
            {action}
          </button>
        </div>
      </div>
    </div>
  );
}

/** "Next that needs you": the next waiting agent after this one, wrapping round. */
function NextNeedsYou({ currentId }: { currentId: string }) {
  const processes = useProcessStore((s) => s.processes);
  const details = useAgentActivityStore((s) => s.details);

  const at = processes.findIndex((p) => p.id === currentId);
  const ordered = [...processes.slice(at + 1), ...processes.slice(0, at + 1)];
  const next = ordered.find(
    (p) =>
      p.id !== currentId &&
      p.kind.kind === "agent" &&
      pillState(p.status, p.kind.kind, details[p.id] ?? UNTRACKED_ACTIVITY) ===
        "needs",
  );
  if (!next) return null;

  return (
    <button
      type="button"
      className={styles.next}
      title={`Next that needs you: ${next.name}`}
      onClick={() => openProcess(next.id)}
    >
      <NextIcon />
      <span className={styles.nextLabel}>Next: {next.name}</span>
    </button>
  );
}
