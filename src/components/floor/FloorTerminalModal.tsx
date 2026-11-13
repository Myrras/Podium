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
import { CloseIcon, RestartIcon, RunIcon, StopIcon } from "../icons";
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
  const removeProcess = useProcessStore((s) => s.removeProcess);
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

  const kind = shown.kind.kind;
  const rename = (name: string | null) => {
    // The store cannot clear a name: a blank one keeps the current name.
    if (name !== null) void renameProcess(shown.id, name);
  };
  const hide = (
    <button
      type="button"
      className="ghost"
      aria-label="Hide terminal"
      title="Hide (keeps running)"
      onClick={close}
    >
      <CloseIcon />
    </button>
  );
  const bar: Bar = subagent
    ? { trailing: hide }
    : kind === "agent"
      ? agentBar(shown, hide)
      : terminalBar(shown, hide);

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
          leading={bar.leading}
          trailing={bar.trailing}
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

  function agentBar(p: ProcessInfo, hideBtn: ReactNode): Bar {
    const ended = p.status.state === "exited";
    return {
      leading: isActive(p.status) ? (
        <IconButton
          label={`Stop ${p.name}`}
          title="Stop"
          onClick={() =>
            setConfirm({
              title: `Stop ${p.name}?`,
              body: "The session ends. You can resume it later.",
              action: "Stop",
              run: () => void stopProcess(p.id),
            })
          }
        >
          <StopIcon />
        </IconButton>
      ) : (
        <IconButton
          label={`Resume ${p.name}`}
          title="Resume session"
          onClick={() => void startProcess(p.id)}
        >
          <RunIcon />
        </IconButton>
      ),
      trailing: (
        <>
          <NextNeedsYou currentId={p.id} />
          {ended && (
            <button
              type="button"
              className="danger"
              onClick={() =>
                setConfirm({
                  title: "Clear the desk?",
                  body: "The agent is removed from the floor.",
                  action: "Clear the desk",
                  run: () => void removeProcess(p.id).then(closeTerminal),
                })
              }
            >
              Clear the desk
            </button>
          )}
          {hideBtn}
        </>
      ),
    };
  }

  function terminalBar(p: ProcessInfo, hideBtn: ReactNode): Bar {
    const active = isActive(p.status);
    const closeFully = async () => {
      if (isActive(p.status)) await stopProcess(p.id);
      await removeProcess(p.id);
      closeTerminal();
    };
    return {
      leading: active ? (
        <IconButton
          label={`Stop ${p.name}`}
          title="Stop"
          onClick={() =>
            setConfirm({
              title: `Stop ${p.name}?`,
              body: "The process stops. You can start it again later.",
              action: "Stop",
              run: () => void stopProcess(p.id),
            })
          }
        >
          <StopIcon />
        </IconButton>
      ) : (
        <IconButton
          label={`Start ${p.name}`}
          title="Start"
          onClick={() => void startProcess(p.id)}
        >
          <RunIcon />
        </IconButton>
      ),
      trailing: (
        <>
          {active && (
            <IconButton
              label={`Restart ${p.name}`}
              title="Restart"
              onClick={() => void restartProcess(p.id)}
            >
              <RestartIcon />
            </IconButton>
          )}
          {p.kind.kind === "terminal" && (
            <button
              type="button"
              className="danger"
              onClick={() =>
                setConfirm({
                  title: `Close ${p.name}?`,
                  body: "The terminal and its output are removed.",
                  action: "Close terminal",
                  run: () => void closeFully(),
                })
              }
            >
              Close terminal
            </button>
          )}
          {hideBtn}
        </>
      ),
    };
  }
}

type Bar = { leading?: ReactNode; trailing: ReactNode };

type Confirm = { title: string; body: string; action: string; run: () => void };

function IconButton({
  label,
  title,
  onClick,
  children,
}: {
  label: string;
  title: string;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      className="ghost"
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
  const setActiveProcess = useProcessStore((s) => s.setActiveProcess);
  const details = useAgentActivityStore((s) => s.details);
  const openTerminal = useFloorStore((s) => s.openTerminal);

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
      onClick={() => {
        openTerminal(next.id);
        setActiveProcess(next.id);
      }}
    >
      Next that needs you: {next.name}
    </button>
  );
}
