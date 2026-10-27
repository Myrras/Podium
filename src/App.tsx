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
  onAssistantEvent,
  onBoardChanged,
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
  onWorktreesChanged,
} from "./ipc/events";
import { backendWslNetworkingMode } from "./ipc/commands";
import { isMac, isWindows } from "./lib/platform";
import {
  applyFontSizeToTerminals,
  applyThemeToTerminals,
} from "./lib/terminalRegistry";
import { subagentProcess } from "./lib/subagents";
import { readTerminalTheme } from "./lib/terminalTheme";
import { startAgentActivityMonitor } from "./state/agentActivityStore";
import { useAssistantStore } from "./state/assistantStore";
import { useIssueStore } from "./state/issueStore";
import { useLayoutStore } from "./state/layoutStore";
import { useProcessStore } from "./state/processStore";
import { useProjectStore } from "./state/projectStore";
import { toastError } from "./state/toastStore";
import { useScratchpadStore } from "./state/scratchpadStore";
import { useSettingsStore } from "./state/settingsStore";
import { useThemeStore } from "./state/themeStore";
import { useTodoStore } from "./state/todoStore";
import { useWorktreeStore } from "./state/worktreeStore";
import { AssistantPane } from "./components/AssistantPane";
import { CloseWarningModal } from "./components/CloseWarningModal";
import { LogoMark } from "./components/LogoMark";
import { GitBoard } from "./components/GitBoard";
import { ScratchpadDetailPane } from "./components/ScratchpadDetailPane";
import { SettingsModal } from "./components/SettingsModal";
import { useSettingsRequestStore } from "./state/settingsRequestStore";
import { Sidebar } from "./components/Sidebar";
import { TerminalPane } from "./components/TerminalPane";
import { TodoDetailPane } from "./components/TodoDetailPane";
import { Toasts } from "./components/Toasts";
import { WelcomeScreen } from "./components/WelcomeScreen";
import { WindowControls } from "./components/WindowControls";
import { AssistantIcon, PowerIcon, SettingsIcon } from "./components/icons";
import styles from "./App.module.css";

export default function App() {
  const [settingsOpen, setSettingsOpen] = useState(false);
  const settingsRequest = useSettingsRequestStore((s) => s.request);
  useEffect(() => {
    if (settingsRequest) setSettingsOpen(true);
  }, [settingsRequest]);
  const [closeWarnOpen, setCloseWarnOpen] = useState(false);

  const sidebarWidth = useLayoutStore((s) => s.sidebarWidth);
  const setSidebarWidth = useLayoutStore((s) => s.setSidebarWidth);
  const openTodo = useLayoutStore((s) => s.openTodo);
  const openScratchpad = useLayoutStore((s) => s.openScratchpad);
  const openBoard = useLayoutStore((s) => s.openBoard);
  const assistantOpen = useLayoutStore((s) => s.assistantOpen);
  const setAssistantOpen = useLayoutStore((s) => s.setAssistantOpen);
  const assistantEnabled = useAssistantStore((s) => s.enabled);
  const assistantWidth = useLayoutStore((s) => s.assistantWidth);
  const setAssistantWidth = useLayoutStore((s) => s.setAssistantWidth);

  const activeProcess = useProcessStore(
    (s) => s.processes.find((p) => p.id === s.activeProcessId) ?? null,
  );
  // A sub-agent opened from the assistant panel: never in the process store,
  // so its terminal pane is fed from the assistant store's snapshot.
  const openSubagentId = useLayoutStore((s) => s.openSubagent);
  const openSubagent = useAssistantStore(
    (s) => s.subagents.find((a) => a.id === openSubagentId) ?? null,
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
    // Loaded here rather than by the panel: the activity monitor and the
    // quit warning need the assistant's sub-agents with the panel closed too.
    void useAssistantStore.getState().loadSubagents();
    const subscriptions: Promise<UnlistenFn>[] = [
      onProcessAdded(() => void useProcessStore.getState().refresh()),
      onProcessRemoved((e) =>
        useProcessStore.getState().applyRemoved(e.processId),
      ),
      onProcessStatus((e) => {
        useProcessStore.getState().applyStatus(e.processId, e.status);
        // Sub-agent processes report their status here too, in between the
        // `subagentsChanged` snapshots (a no-op for any other process).
        useAssistantStore.getState().applySubagentStatus(e.processId, e.status);
      }),
      onProcessUpdated(() => void useProcessStore.getState().refresh()),
      onProjectOpened((e) => {
        void useProjectStore.getState().refresh();
        void useProjectStore.getState().refreshRecents();
        // Background-load worktrees so the modal/picker render instantly
        // instead of showing "Loading…" on first open.
        void useWorktreeStore.getState().refresh(e.projectId);
      }),
      onProjectUpdated(() => void useProjectStore.getState().refresh()),
      onProjectClosed((e) => {
        void useProjectStore.getState().refresh();
        void useProcessStore.getState().refresh();
        useTodoStore.getState().dropProject(e.projectId);
        useScratchpadStore.getState().dropProject(e.projectId);
        useWorktreeStore.getState().dropProject(e.projectId);
        useIssueStore.getState().dropProject(e.projectId);
        const layout = useLayoutStore.getState();
        if (layout.openBoard?.projectId === e.projectId)
          layout.clearOpenBoard();
      }),
      onTodosChanged((e) => void useTodoStore.getState().refresh(e.projectId)),
      onScratchpadsChanged(
        (e) => void useScratchpadStore.getState().refresh(e.projectId),
      ),
      onWorktreesChanged(
        (e) => void useWorktreeStore.getState().refresh(e.projectId),
      ),
      onBoardChanged((e) => void useIssueStore.getState().hydrate(e.projectId)),
      // The assistant's chat stream (its headless session's output).
      onAssistantEvent((e) => useAssistantStore.getState().apply(e)),
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

  // Re-pull the sub-agent list whenever the window comes back into view: a
  // webview held back while hidden may have missed snapshots, and the panel
  // should be right the moment the user looks.
  useEffect(() => {
    const resync = () => {
      if (document.visibilityState === "visible") {
        void useAssistantStore.getState().loadSubagents();
      }
    };
    window.addEventListener("focus", resync);
    document.addEventListener("visibilitychange", resync);
    return () => {
      window.removeEventListener("focus", resync);
      document.removeEventListener("visibilitychange", resync);
    };
  }, []);

  // Poll running agents for their activity (working / waiting / idle) and
  // raise the "needs input" alert on transitions. One loop for the whole app,
  // so the alert fires once no matter how many views show the agent.
  useEffect(() => startAgentActivityMonitor(), []);

  // Load the Responses list once at launch, even with the assistant pane
  // closed: it is the baseline for "new item" desktop notifications, so what
  // the persisted list already holds never pings.
  useEffect(() => {
    void useAssistantStore.getState().loadResponses();
    void useAssistantStore.getState().loadEnabled();
  }, []);

  // Terminals are not React-rendered, so push theme/font changes into the
  // registry. `mode` flips after the `data-theme` attribute is applied, so
  // reading the tokens here always sees the new palette.
  const themeMode = useThemeStore((s) => s.mode);
  useEffect(() => {
    applyThemeToTerminals(readTerminalTheme());
  }, [themeMode]);

  // WSL-backend mode: agents inside WSL reach Podium's MCP server on
  // 127.0.0.1 only with mirrored networking. Say so once if it's off.
  useEffect(() => {
    if (!isWindows) return;
    let cancelled = false;
    backendWslNetworkingMode()
      .then((mode) => {
        if (cancelled || !mode || mode === "mirrored") return;
        toastError(
          "WSL networking is not mirrored",
          `Agents in WSL can't reach Podium's MCP tools (networking mode: ${mode}). ` +
            "Set networkingMode=mirrored under [wsl2] in %UserProfile%\\.wslconfig, " +
            "run `wsl --shutdown`, and restart Podium.",
          { sticky: true },
        );
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, []);

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

  // Same drag pattern as the sidebar, but the assistant sits on the right:
  // dragging left makes it wider.
  const onAssistantResizerDown = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      const startX = e.clientX;
      const startW = assistantWidth;
      function onMove(ev: MouseEvent) {
        setAssistantWidth(startW - (ev.clientX - startX));
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
    [assistantWidth, setAssistantWidth],
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
        className={`${styles.titlebar}${isWindows ? ` ${styles.titlebarWindows}` : ""}${isMac ? ` ${styles.titlebarMac}` : ""}`}
        // Windows has no native title bar (decorations: false) and macOS
        // overlays its traffic lights on our strip (titleBarStyle: Overlay) —
        // see CLAUDE.md's "Window chrome" — so the whole strip is the drag
        // handle on either. Linux keeps its native title bar for now.
        data-tauri-drag-region={isWindows || isMac ? true : undefined}
      >
        {/* macOS draws its own traffic lights top-left (.titlebarMac leaves
            room for them), so the brand is centered instead of leading. */}
        <div
          className={`${styles.brand}${isMac ? ` ${styles.brandCentered}` : ""}`}
        >
          <LogoMark size={18} className={styles.logo} aria-hidden />
          <span className={styles.product}>Podium</span>
          <span className={styles.tagline}>agent console</span>
        </div>
        <div className={styles.titlebarRight}>
          <button
            type="button"
            className={styles.settingsBtn}
            aria-label="Assistant"
            title="Assistant"
            aria-pressed={assistantOpen}
            onClick={() => setAssistantOpen(!assistantOpen)}
          >
            <AssistantIcon size={16} />
          </button>
          <button
            type="button"
            className={`${styles.settingsBtn} ${styles.powerBtn}`}
            aria-label={
              assistantEnabled ? "Turn assistant off" : "Turn assistant on"
            }
            title={
              assistantEnabled ? "Turn assistant off" : "Turn assistant on"
            }
            aria-pressed={assistantEnabled}
            onClick={() =>
              void useAssistantStore.getState().setEnabled(!assistantEnabled)
            }
          >
            <PowerIcon size={16} />
          </button>
          {/* macOS reaches Settings via the native menu bar (⌘,); the gear
              lives in the custom title bar on the other platforms. */}
          {!isMac && (
            <button
              type="button"
              className={styles.settingsBtn}
              aria-label="Settings"
              title="Settings"
              onClick={() => setSettingsOpen(true)}
            >
              <SettingsIcon size={16} />
            </button>
          )}
          {isWindows && <WindowControls />}
        </div>
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
          {openBoard ? (
            <GitBoard
              key={openBoard.projectId}
              projectId={openBoard.projectId}
            />
          ) : openScratchpad ? (
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
          ) : openSubagent ? (
            <TerminalPane
              key={openSubagent.id}
              process={subagentProcess(openSubagent)}
            />
          ) : activeProcess ? (
            <TerminalPane process={activeProcess} />
          ) : (
            <WelcomeScreen />
          )}
        </main>
        {assistantOpen && (
          <>
            <div
              className={styles.sidebarResizer}
              onMouseDown={onAssistantResizerDown}
              role="separator"
              aria-orientation="vertical"
              aria-label="Resize assistant"
            />
            <AssistantPane />
          </>
        )}
      </div>

      <Toasts />
      <SettingsModal
        open={settingsOpen}
        onClose={() => {
          setSettingsOpen(false);
          useSettingsRequestStore.getState().clear();
        }}
      />
      <CloseWarningModal
        open={closeWarnOpen}
        onClose={() => setCloseWarnOpen(false)}
      />
    </div>
  );
}
