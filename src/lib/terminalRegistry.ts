/**
 * Registry of live xterm.js terminals, keyed by process id.
 *
 * Terminals live *outside* the React tree: a `Terminal` instance (and its
 * scrollback, cursor state, etc.) survives tab switches and layout changes —
 * React components only reparent the terminal's DOM element. Each entry also
 * owns its IPC attachment (snapshot + batched live stream).
 *
 * ## Stream handling
 * On attach the backend sends a `snapshot` (full scrollback, base64) whose
 * `seq` is the first live sequence that follows; each `data` batch carries
 * the seq of its first chunk. Any batch with `seq < nextSeq` is stale
 * (already covered by the snapshot) and is dropped. A `lagged` event means
 * bytes were lost — we re-attach for a fresh snapshot; a bumped generation
 * counter makes messages from the abandoned channel no-ops.
 */

import { Terminal, type ITheme } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";

import {
  processAttach,
  processResize,
  processWrite,
  toIpcError,
} from "../ipc/commands";
import type { ProcessId, TermEvent } from "../ipc/types";
import { logWarning } from "./log";
import { base64ToBytes, encodeInput } from "./termProtocol";

export interface TerminalOptions {
  fontSize: number;
  fontFamily: string;
  theme: ITheme;
}

interface Entry {
  terminal: Terminal;
  fit: FitAddon;
  nextSeq: number;
  /** Bumped on every (re-)attach; stale channel callbacks check it. */
  generation: number;
  /** Unix ms of the last output bytes (backs the agent "working" heuristic). */
  lastOutputAt: number | null;
}

const entries = new Map<ProcessId, Entry>();

/** Keystroke-write failures are expected when the process just exited. */
function writeInput(processId: ProcessId, data: string): void {
  processWrite(processId, encodeInput(data)).catch((e: unknown) => {
    const err = toIpcError(e);
    if (err.kind !== "processNotRunning" && err.kind !== "processNotFound") {
      logWarning("terminal input", err.message);
    }
  });
}

function handleTermEvent(
  processId: ProcessId,
  generation: number,
  event: TermEvent,
): void {
  const entry = entries.get(processId);
  if (!entry || entry.generation !== generation) return; // stale channel
  switch (event.type) {
    case "snapshot":
      entry.terminal.reset();
      entry.terminal.write(base64ToBytes(event.dataB64));
      entry.nextSeq = event.seq;
      if (event.dataB64.length > 0) entry.lastOutputAt = Date.now();
      break;
    case "data":
      if (event.seq < entry.nextSeq) return; // covered by the snapshot
      entry.terminal.write(base64ToBytes(event.dataB64));
      entry.nextSeq = event.seq + 1;
      entry.lastOutputAt = Date.now();
      break;
    case "lagged":
      // Bytes were dropped server-side; a partial stream would corrupt the
      // terminal. Re-attach for a clean snapshot.
      attach(processId);
      break;
  }
}

function attach(processId: ProcessId): void {
  const entry = entries.get(processId);
  if (!entry) return;
  entry.generation += 1;
  const generation = entry.generation;
  processAttach(processId, (event) =>
    handleTermEvent(processId, generation, event),
  ).catch((e: unknown) => {
    logWarning("terminal attach", toIpcError(e).message);
  });
}

/** Get (or lazily create + attach) the terminal for a process. */
export function acquireTerminal(
  processId: ProcessId,
  options: TerminalOptions,
): Terminal {
  const existing = entries.get(processId);
  if (existing) return existing.terminal;

  const terminal = new Terminal({
    allowProposedApi: true,
    convertEol: false,
    cursorBlink: true,
    fontFamily: options.fontFamily,
    fontSize: options.fontSize,
    scrollback: 10_000,
    theme: options.theme,
  });
  const fit = new FitAddon();
  terminal.loadAddon(fit);
  terminal.onData((data) => writeInput(processId, data));

  entries.set(processId, {
    terminal,
    fit,
    nextSeq: 0,
    generation: 0,
    lastOutputAt: null,
  });
  attach(processId);
  return terminal;
}

/**
 * Unix ms of the last output bytes for a process, or `null` when it has no
 * live terminal / no output yet. Polled by `useAgentActivity` for the
 * "working" heuristic — deliberately a plain accessor, not a subscription.
 */
export function getLastOutputAt(processId: ProcessId): number | null {
  return entries.get(processId)?.lastOutputAt ?? null;
}

/**
 * The plain text currently on screen (the visible viewport, trailing
 * whitespace trimmed) for a process, or `null` when it has no live terminal.
 * Backs the "needs input" heuristic — the activity store scans this for
 * permission/confirmation prompts (see `detectInputPrompt`). Reads the live
 * xterm buffer, so like `attachToElement` it only works with a real browser.
 */
export function readViewportText(processId: ProcessId): string | null {
  const entry = entries.get(processId);
  if (!entry) return null;
  const { terminal } = entry;
  const buffer = terminal.buffer.active;
  const lines: string[] = [];
  for (let row = 0; row < terminal.rows; row += 1) {
    const line = buffer.getLine(buffer.baseY + row);
    if (line) lines.push(line.translateToString(true));
  }
  return lines.join("\n");
}

/**
 * Reparent `element` into `container`, evicting any other terminal element
 * left there. The host div is reused across process switches (TerminalView
 * keeps the same element and only swaps `processId`), so a plain `appendChild`
 * would leave the previous process's terminal visible behind the new one.
 * Exported for unit testing (the `terminal.open` path needs a real browser).
 */
export function reparentTerminalElement(
  element: HTMLElement,
  container: HTMLElement,
): void {
  // Already the sole child of this host — nothing to do (avoids DOM churn on
  // every re-fit). Otherwise `replaceChildren` moves ours in and drops the rest.
  if (element.parentNode === container && container.childElementCount === 1) {
    return;
  }
  container.replaceChildren(element);
}

/** Mount the terminal into `container` (first open) or reparent it there. */
export function attachToElement(
  processId: ProcessId,
  container: HTMLElement,
): void {
  const entry = entries.get(processId);
  if (!entry) return;
  const { terminal } = entry;
  if (terminal.element) {
    reparentTerminalElement(terminal.element, container);
  } else {
    // First open: clear any stale terminal already parented in this reused
    // host before xterm appends ours.
    container.replaceChildren();
    terminal.open(container);
  }
}

/** Refit to the container and propagate the new grid size to the PTY. */
export function fitTerminal(processId: ProcessId): void {
  const entry = entries.get(processId);
  if (!entry?.terminal.element) return;
  entry.fit.fit();
  const { cols, rows } = entry.terminal;
  processResize(processId, cols, rows).catch(() => {
    // Expected while the process is not running; the PTY is resized on the
    // next successful fit after a start.
  });
}

/** Dispose the terminal and forget the process (e.g. on process removal). */
export function disposeTerminal(processId: ProcessId): void {
  const entry = entries.get(processId);
  if (!entry) return;
  entry.generation += 1; // silence any in-flight channel messages
  entries.delete(processId);
  entry.terminal.dispose();
}

/** Apply a new theme to every live terminal (theme switch). */
export function applyThemeToTerminals(theme: ITheme): void {
  for (const entry of entries.values()) {
    entry.terminal.options.theme = theme;
  }
}

/** Apply a new font size to every live terminal, refitting each. */
export function applyFontSizeToTerminals(fontSize: number): void {
  for (const [processId, entry] of entries) {
    entry.terminal.options.fontSize = fontSize;
    if (entry.terminal.element) fitTerminal(processId);
  }
}
