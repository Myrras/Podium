/**
 * The terminal of a desk (or an assistant sub-agent) in an overlay over the
 * floor. Not `Modal`: Esc must reach the terminal (Claude Code uses it), and
 * the card may not scale, which would break xterm's mouse mapping.
 */

import { useEffect, useRef, useState } from "react";
import type { KeyboardEvent } from "react";
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
import { CloseIcon } from "../icons";
import { TerminalPane } from "../TerminalPane";
import styles from "./FloorTerminalModal.module.css";

const FOCUSABLE =
  'button:not([disabled]), input, textarea, [href], [tabindex]:not([tabindex="-1"])';

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

  // Tab stays inside the card. A focused terminal never gets here: xterm
  // cancels Tab (and Esc) itself and sends it to the PTY.
  const trapTab = (e: KeyboardEvent<HTMLDivElement>) => {
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
  };

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
        <header className={styles.header}>
          {subagent ? (
            <span className={styles.title}>{shown.name}</span>
          ) : (
            <AgentActions process={shown} onCleared={closeTerminal} />
          )}
          <span className={styles.end}>
            {!subagent && <NextNeedsYou currentId={shown.id} />}
            <button
              type="button"
              className="ghost"
              aria-label="Close terminal"
              onClick={close}
            >
              <CloseIcon />
            </button>
          </span>
        </header>
        <TerminalPane process={shown} />
      </div>
    </div>,
    document.body,
  );
}

/** The name (click to rename) and, for an agent, its session actions. */
function AgentActions({
  process,
  onCleared,
}: {
  process: ProcessInfo;
  onCleared: () => void;
}) {
  const renameProcess = useProcessStore((s) => s.renameProcess);
  const startProcess = useProcessStore((s) => s.startProcess);
  const stopProcess = useProcessStore((s) => s.stopProcess);
  const removeProcess = useProcessStore((s) => s.removeProcess);
  const [draft, setDraft] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const editing = draft !== null;

  useEffect(() => {
    if (editing) inputRef.current?.select();
  }, [editing]);

  if (process.kind.kind !== "agent") {
    return <span className={styles.title}>{process.name}</span>;
  }

  const commit = () => {
    const next = draft?.trim() ?? "";
    setDraft(null);
    if (next.length > 0 && next !== process.name) {
      void renameProcess(process.id, next);
    }
  };
  const ended = process.status.state === "exited";

  return (
    <>
      {editing ? (
        <input
          ref={inputRef}
          className={styles.nameInput}
          aria-label="Agent name"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              commit();
            } else if (e.key === "Escape") {
              // Cancels the edit only; Esc never closes the modal.
              e.preventDefault();
              setDraft(null);
            }
          }}
        />
      ) : (
        <button
          type="button"
          className={`ghost ${styles.title}`}
          title="Rename"
          onClick={() => setDraft(process.name)}
        >
          {process.name}
        </button>
      )}
      {isActive(process.status) && (
        <button type="button" onClick={() => void stopProcess(process.id)}>
          Stop
        </button>
      )}
      {ended && (
        <>
          <button
            type="button"
            className="primary"
            onClick={() => void startProcess(process.id)}
          >
            Resume session
          </button>
          <button
            type="button"
            onClick={() => {
              void removeProcess(process.id).then(onCleared);
            }}
          >
            Clear the desk
          </button>
        </>
      )}
    </>
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
