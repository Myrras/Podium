/**
 * Mounts a registry-owned xterm.js terminal into the React tree.
 *
 * The `Terminal` instance itself lives in `lib/terminalRegistry` and survives
 * unmounts — this component only (re)parents the terminal's DOM element into
 * its host div and keeps the grid fitted to the host's size. Switching
 * `processId` swaps which terminal is parented here without losing any
 * scrollback or cursor state.
 */

import { useEffect, useRef } from "react";

import "@xterm/xterm/css/xterm.css";

import type { ProcessId } from "../ipc/types";
import {
  acquireTerminal,
  attachToElement,
  fitTerminal,
} from "../lib/terminalRegistry";
import {
  readTerminalFontFamily,
  readTerminalTheme,
} from "../lib/terminalTheme";
import { useSettingsStore } from "../state/settingsStore";
import styles from "./TerminalView.module.css";

export function TerminalView({ processId }: { processId: ProcessId }) {
  const hostRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const terminal = acquireTerminal(processId, {
      fontSize: useSettingsStore.getState().terminal.fontSize,
      fontFamily: readTerminalFontFamily(),
      theme: readTerminalTheme(),
    });
    attachToElement(processId, host);
    // Defer the first fit to the next frame: running it synchronously here
    // measures the host before the just-mounted pane's flex layout is final
    // (the header bar's height not yet subtracted), yielding one row too many
    // and clipping the terminal's bottom line. rAF measures the settled size.
    const raf = requestAnimationFrame(() => {
      fitTerminal(processId);
      terminal.focus();
    });

    const observer = new ResizeObserver(() => fitTerminal(processId));
    observer.observe(host);
    return () => {
      cancelAnimationFrame(raf);
      observer.disconnect();
    };
  }, [processId]);

  return <div ref={hostRef} className={styles.host} />;
}
