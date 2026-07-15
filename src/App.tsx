/**
 * Podium — application shell.
 *
 * Layout: a slim title bar on top; below it a two-column body — the entity
 * sidebar (Agents / Processes / Terminals) on the left and the work area on
 * the right (the focused process's terminal, or the welcome screen). This
 * component also owns the app-level IPC wiring: the initial state pull, the
 * lifecycle-event subscriptions, and pushing theme/font changes into the
 * live terminals.
 */

import { useCallback, useEffect, useState } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";

import {
  onMenuOpenSettings,
  onProcessAdded,
  onProcessRemoved,
  onProcessStatus,
  onProcessUpdated,
  onProjectClosed,
  onProjectOpened,
  onProjectUpdated,
  onScratchpadsChanged,
  onTodosChanged,
  onWindowCloseRequested,
} from "./ipc/events";
import { isMac, isWindows } from "./lib/platform";
import {
  applyFontSizeToTerminals,
  applyThemeToTerminals,
} from "./lib/terminalRegistry";
import { readTerminalTheme } from "./lib/terminalTheme";
import { startAgentActivityMonitor } from "./state/agentActivityStore";
import { useLayoutStore } from "./state/layoutStore";
import { useProcessStore } from "./state/processStore";
import { useProjectStore } from "./state/projectStore";
import { useScratchpadStore } from "./state/scratchpadStore";
import { useSettingsStore } from "./state/settingsStore";
import { useThemeStore } from "./state/themeStore";
import { useTodoStore } from "./state/todoStore";
import { CloseWarningModal } from "./components/CloseWarningModal";
import { LogoMark } from "./components/LogoMark";
import { ScratchpadDetailPane } from "./components/ScratchpadDetailPane";
import { SettingsModal } from "./components/SettingsModal";
import { Sidebar } from "./components/Sidebar";
import { TerminalPane } from "./components/TerminalPane";
import { TodoDetailPane } from "./components/TodoDetailPane";
import { Toasts } from "./components/Toasts";
import { WelcomeScreen } from "./components/WelcomeScreen";
import { WindowControls } from "./components/WindowControls";
import { SettingsIcon } from "./components/icons";
import styles from "./App.module.css";

export default function App() {
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [closeWarnOpen, setCloseWarnOpen] = useState(false);

  const sidebarWidth = useLayoutStore((s) => s.sidebarWidth);
  const setSidebarWidth = useLayoutStore((s) => s.setSidebarWidth);
  const openTodo = useLayoutStore((s) => s.openTodo);
  const openScratchpad = useLayoutStore((s) => s.openScratchpad);

  const activeProcess = useProcessStore(
    (s) => s.processes.find((p) => p.id === s.activeProcessId) ?? null,
  );

  // Initial state pull + lifecycle-event subscriptions. Startup restores the
  // persisted workspace (re-opening every project), then re-pulls processes.
  // Mutations elsewhere (this UI, agents via MCP, project close) all flow
  // back through these events, so this is the single place the stores learn
  // about changes.
  useEffect(() => {
    void useProjectStore
      .getState()
      .restoreWorkspace()
      .then(() => useProcessStore.getState().refresh());
    void useProjectStore.getState().refreshRecents();
    void useProcessStore.getState().refresh();
    const subscriptions: Promise<UnlistenFn>[] = [
      onProcessAdded(() => void useProcessStore.getState().refresh()),
      onProcessRemoved((e) =>
        useProcessStore.getState().applyRemoved(e.processId),
      ),
      onProcessStatus((e) =>
        useProcessStore.getState().applyStatus(e.processId, e.status),
      ),
      onProcessUpdated(() => void useProcessStore.getState().refresh()),
      onProjectOpened(() => {
        void useProjectStore.getState().refresh();
        void useProjectStore.getState().refreshRecents();
      }),
      onProjectUpdated(() => void useProjectStore.getState().refresh()),
      onProjectClosed((e) => {
        void useProjectStore.getState().refresh();
        void useProcessStore.getState().refresh();
        useTodoStore.getState().dropProject(e.projectId);
        useScratchpadStore.getState().dropProject(e.projectId);
      }),
      onTodosChanged((e) => void useTodoStore.getState().refresh(e.projectId)),
      onScratchpadsChanged(
        (e) => void useScratchpadStore.getState().refresh(e.projectId),
      ),
      // The backend blocked a quit because agents/terminals are still
      // running — surface the warning so the user can confirm or cancel.
      onWindowCloseRequested(() => setCloseWarnOpen(true)),
    ];
    return () => {
      for (const sub of subscriptions) {
        sub.then((unlisten) => unlisten()).catch(() => undefined);
      }
    };
  }, []);

  // Poll running agents for their activity (working / waiting / idle) and
  // raise the "needs input" alert on transitions. One loop for the whole app,
  // so the alert fires once no matter how many views show the agent.
  useEffect(() => startAgentActivityMonitor(), []);

  // Terminals are not React-rendered, so push theme/font changes into the
  // registry. `mode` flips after the `data-theme` attribute is applied, so
  // reading the tokens here always sees the new palette.
  const themeMode = useThemeStore((s) => s.mode);
  useEffect(() => {
    applyThemeToTerminals(readTerminalTheme());
  }, [themeMode]);

  const termFontSize = useSettingsStore((s) => s.terminal.fontSize);
  useEffect(() => {
    applyFontSizeToTerminals(termFontSize);
  }, [termFontSize]);

  const onSidebarResizerDown = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      const startX = e.clientX;
      const startW = sidebarWidth;
      function onMove(ev: MouseEvent) {
        setSidebarWidth(startW + (ev.clientX - startX));
      }
      function onUp() {
        window.removeEventListener("mousemove", onMove);
        window.removeEventListener("mouseup", onUp);
        document.body.style.cursor = "";
      }
      document.body.style.cursor = "col-resize";
      window.addEventListener("mousemove", onMove);
      window.addEventListener("mouseup", onUp);
    },
    [sidebarWidth, setSidebarWidth],
  );

  // Suppress the webview's default right-click menu (the browser-y "Save as /
  // Print / Share" items, most glaring on Windows' WebView2). Kept inside
  // editable surfaces so native copy/paste still works there.
  useEffect(() => {
    const onContextMenu = (e: MouseEvent) => {
      const target = e.target as HTMLElement | null;
      if (
        target?.closest(
          'input, textarea, [contenteditable=""], [contenteditable="true"]',
        )
      ) {
        return;
      }
      e.preventDefault();
    };
    window.addEventListener("contextmenu", onContextMenu);
    return () => window.removeEventListener("contextmenu", onContextMenu);
  }, []);

  // Settings access. macOS: the native menu's Settings… item (⌘,) emits
  // `menu:open-settings`, so the in-app shortcut binds only elsewhere to
  // avoid a double-fire. A desktop app must never reload its webview, so
  // swallow the browser refresh shortcuts (F5, Cmd/Ctrl+R) on every platform.
  useEffect(() => {
    const menuSub = onMenuOpenSettings(() => setSettingsOpen(true));
    const onKey = (e: KeyboardEvent) => {
      if (!isMac && (e.ctrlKey || e.metaKey) && e.key === ",") {
        e.preventDefault();
        setSettingsOpen(true);
        return;
      }
      if (
        e.key === "F5" ||
        ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "r")
      ) {
        e.preventDefault();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      menuSub.then((unlisten) => unlisten()).catch(() => undefined);
    };
  }, []);

  return (
    <div className={styles.app}>
      <header
        className={`${styles.titlebar}${isWindows ? ` ${styles.titlebarWindows}` : ""}`}
        // Windows has no native title bar (decorations: false); the whole strip
        // becomes the drag handle. macOS keeps its native title bar, so no
        // drag region there.
        data-tauri-drag-region={isWindows ? true : undefined}
      >
        <div className={styles.brand}>
          <LogoMark size={18} className={styles.logo} aria-hidden />
          <span className={styles.product}>Podium</span>
          <span className={styles.tagline}>agent console</span>
        </div>
        {/* macOS reaches Settings via the native menu bar (⌘,); the gear
            lives in the custom title bar on the other platforms. */}
        {!isMac && (
          <div className={styles.titlebarRight}>
            <button
              type="button"
              className={styles.settingsBtn}
              aria-label="Settings"
              title="Settings"
              onClick={() => setSettingsOpen(true)}
            >
              <SettingsIcon size={16} />
            </button>
            {isWindows && <WindowControls />}
          </div>
        )}
      </header>

      <div className={styles.body}>
        <Sidebar />
        <div
          className={styles.sidebarResizer}
          onMouseDown={onSidebarResizerDown}
          role="separator"
          aria-orientation="vertical"
          aria-label="Resize sidebar"
        />
        <main className={styles.work}>
          {openScratchpad ? (
            <ScratchpadDetailPane
              key={openScratchpad.scratchpadId}
              projectId={openScratchpad.projectId}
              scratchpadId={openScratchpad.scratchpadId}
            />
          ) : openTodo ? (
            <TodoDetailPane
              key={openTodo.todoId}
              projectId={openTodo.projectId}
              todoId={openTodo.todoId}
            />
          ) : activeProcess ? (
            <TerminalPane process={activeProcess} />
          ) : (
            <WelcomeScreen />
          )}
        </main>
      </div>

      <Toasts />
      <SettingsModal
        open={settingsOpen}
        onClose={() => setSettingsOpen(false)}
      />
      <CloseWarningModal
        open={closeWarnOpen}
        onClose={() => setCloseWarnOpen(false)}
      />
    </div>
  );
}
