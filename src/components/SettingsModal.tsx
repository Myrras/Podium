/**
 * App settings dialog with three tabs: General (appearance, terminal), Agents
 * (per-adapter command override + default arguments, plus the argument merge
 * mode), and MCP (one-click registration of the stdio bridge with external
 * clients like Claude Code and Auggie).
 */

import { useEffect, useRef, useState } from "react";

import { Modal } from "./Modal";
import { useThemeStore, type ThemeMode } from "../state/themeStore";
import { useSettingsStore } from "../state/settingsStore";
import {
  agentSettingsGet,
  agentSettingsSetAdapter,
  agentSettingsSetDefaultAdapter,
  agentSettingsSetMergeMode,
  agentSettingsSetSuggestWorktree,
  mcpClientInstall,
  mcpClientsStatus,
  toIpcError,
} from "../ipc/commands";
import type {
  AgentAdapterConfig,
  AgentSettingsDto,
  McpClientInfo,
  MergeMode,
} from "../ipc/types";
import { toastSuccess, toastError } from "../state/toastStore";
import { playNotifySound } from "../lib/notify";
import { AddIcon, CheckIcon, CopyIcon, MinusIcon } from "./icons";
import styles from "./SettingsModal.module.css";

interface SettingsModalProps {
  open: boolean;
  onClose: () => void;
}

type SettingsTab = "general" | "agents" | "mcp";

const THEMES: {
  mode: ThemeMode;
  label: string;
  swatches: [string, string, string];
}[] = [
  { mode: "dark", label: "Dark", swatches: ["#0e1116", "#4493f8", "#e6edf3"] },
  {
    mode: "light",
    label: "Light",
    swatches: ["#f6f8fa", "#0969da", "#1f2328"],
  },
  {
    mode: "retro",
    label: "Retro",
    swatches: ["#f2e5bc", "#458588", "#3c3836"],
  },
];

const FONT_SIZE_MIN = 10;
const FONT_SIZE_MAX = 24;

// Placeholder for the terminal-shell field: the platform default the backend
// falls back to when the field is blank.
const DEFAULT_SHELL_HINT = navigator.userAgent.includes("Windows")
  ? "powershell (default)"
  : "$SHELL (default)";

/** Checkbox toggle row with label and optional helper text. */
function SettingToggle({
  label,
  help,
  value,
  onChange,
}: {
  label: string;
  help?: string;
  value: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <div className={styles.row}>
      <span className={styles.rowLabel}>
        {label}
        {help && <small className={styles.rowHelp}>{help}</small>}
      </span>
      <button
        type="button"
        role="switch"
        aria-checked={value}
        className={styles.toggle}
        data-on={value ? "true" : "false"}
        onClick={() => onChange(!value)}
        aria-label={label}
      />
    </div>
  );
}

/** One external client card: status, install command, Run + Copy buttons. */
function McpClientCard({
  client,
  onClients,
}: {
  client: McpClientInfo;
  onClients: (clients: McpClientInfo[]) => void;
}) {
  const [installing, setInstalling] = useState(false);

  return (
    <div className={styles.clientCard}>
      <div className={styles.clientHeader}>
        <span className={styles.clientName}>{client.displayName}</span>
        <span
          className={styles.clientBadge}
          data-installed={client.installed ? "true" : "false"}
        >
          <span className={styles.clientBadgeDot} aria-hidden />
          {client.installed
            ? "Installed"
            : client.cliAvailable
              ? "Not installed"
              : "CLI not found"}
        </span>
      </div>
      <small className={styles.rowHelp}>
        {client.installed
          ? "Podium is registered — agents in this client can use Podium's MCP tools."
          : client.cliAvailable
            ? "Run the command below (or press Run) to register Podium."
            : "Install the client's CLI first, then register Podium here."}
      </small>
      <div className={styles.clientCommandRow}>
        <code className={styles.clientCommand}>{client.installCommand}</code>
        <div className={styles.clientActions}>
          <button
            type="button"
            disabled={!client.cliAvailable || installing}
            onClick={() => {
              setInstalling(true);
              mcpClientInstall(client.id)
                .then((clients) => {
                  onClients(clients);
                  toastSuccess(`Podium registered with ${client.displayName}`);
                })
                .catch((e) =>
                  toastError(
                    "Failed to register Podium",
                    toIpcError(e).message,
                  ),
                )
                .finally(() => setInstalling(false));
            }}
          >
            {installing ? "Running…" : "Run"}
          </button>
          <button
            type="button"
            aria-label="Copy install command"
            title="Copy install command"
            onClick={() => {
              navigator.clipboard
                .writeText(client.installCommand)
                .then(() => toastSuccess("Command copied"))
                .catch(() => toastError("Failed to copy to clipboard"));
            }}
          >
            <CopyIcon />
          </button>
        </div>
      </div>
      <small className={styles.rowHelp}>
        Check with <code>{client.checkCommand}</code>. If a stale{" "}
        <code>podium</code> entry exists, Run replaces it automatically.
      </small>
    </div>
  );
}

export function SettingsModal({ open, onClose }: SettingsModalProps) {
  const [tab, setTab] = useState<SettingsTab>("general");
  const [clients, setClients] = useState<McpClientInfo[] | null>(null);

  // Refresh the client registration snapshot every time the dialog opens.
  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    mcpClientsStatus()
      .then((list) => {
        if (!cancelled) setClients(list);
      })
      .catch(() => {
        if (!cancelled) setClients([]);
      });
    return () => {
      cancelled = true;
    };
  }, [open]);

  return (
    <Modal open={open} title="Settings" onClose={onClose} width={520}>
      {/* Tabs */}
      <div className={styles.tabs} role="tablist" aria-label="Settings tabs">
        {(
          [
            ["general", "General"],
            ["agents", "Agents"],
            ["mcp", "MCP"],
          ] as [SettingsTab, string][]
        ).map(([id, label]) => (
          <button
            key={id}
            type="button"
            role="tab"
            aria-selected={tab === id}
            className={styles.tab}
            data-active={tab === id ? "true" : "false"}
            onClick={() => setTab(id)}
          >
            {label}
          </button>
        ))}
      </div>

      {tab === "mcp" ? (
        <section className={styles.section}>
          <h3 className={styles.sectionLabel}>External clients</h3>
          <small className={styles.rowHelp}>
            Register Podium's MCP server so agents in these clients can list
            processes, read output and spawn sibling agents.
          </small>
          {clients === null ? (
            <small className={styles.rowHelp}>Checking clients…</small>
          ) : clients.length === 0 ? (
            <small className={styles.rowHelp}>
              Could not probe external clients.
            </small>
          ) : (
            clients.map((client) => (
              <McpClientCard
                key={client.id}
                client={client}
                onClients={setClients}
              />
            ))
          )}
        </section>
      ) : tab === "agents" ? (
        <AgentsTab />
      ) : (
        <GeneralTab />
      )}
    </Modal>
  );
}

const MERGE_MODES: { value: MergeMode; label: string }[] = [
  { value: "merge", label: "Merge — global args, then project args" },
  { value: "project-overrides", label: "Project overrides global" },
  { value: "global-overrides", label: "Global overrides project" },
];

/** Agents tab: per-adapter command + default arguments, plus the merge mode. */
function AgentsTab() {
  const [data, setData] = useState<AgentSettingsDto | null>(null);
  /** Adapter id currently being edited, or `null` for the list view. */
  const [editing, setEditing] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    agentSettingsGet()
      .then((d) => {
        if (!cancelled) setData(d);
      })
      .catch(() => {
        if (!cancelled)
          setData({
            mergeMode: "merge",
            defaultAdapter: "",
            suggestWorktree: true,
            adapters: [],
          });
      });
    return () => {
      cancelled = true;
    };
  }, []);

  if (data === null) {
    return (
      <section className={styles.section}>
        <small className={styles.rowHelp}>Loading agents…</small>
      </section>
    );
  }

  const editingAdapter =
    editing !== null ? data.adapters.find((a) => a.id === editing) : undefined;
  if (editingAdapter) {
    return (
      <AgentEditForm
        adapter={editingAdapter}
        onCancel={() => setEditing(null)}
        onSaved={(next) => {
          setData(next);
          setEditing(null);
        }}
      />
    );
  }

  // The built-in fallback when no default is pinned. Kept in sync with the
  // core's DEFAULT_ADAPTER_ID.
  const BUILTIN_DEFAULT = "claude-code";
  const effectiveDefault = data.defaultAdapter || BUILTIN_DEFAULT;

  return (
    <>
      <section className={styles.section}>
        <h3 className={styles.sectionLabel}>Default agent</h3>
        <small className={styles.rowHelp}>
          Which agent a bare spawn (the to-do agent button, or agents spawning
          agents over MCP) uses. A project's <code>podium.yml</code>{" "}
          <code>agents.default_adapter</code> overrides this per project.
        </small>
        <select
          aria-label="Default agent adapter"
          value={effectiveDefault}
          onChange={(e) => {
            const id = e.target.value;
            setData({ ...data, defaultAdapter: id });
            agentSettingsSetDefaultAdapter(id)
              .then(setData)
              .catch((err) =>
                toastError(
                  "Could not save default agent",
                  toIpcError(err).message,
                ),
              );
          }}
        >
          {data.adapters.map((a) => (
            <option key={a.id} value={a.id}>
              {a.displayName}
              {a.available ? "" : " (not installed)"}
            </option>
          ))}
        </select>
      </section>

      <section className={styles.section}>
        <h3 className={styles.sectionLabel}>Argument merge</h3>
        <small className={styles.rowHelp}>
          How global default arguments combine with a project's{" "}
          <code>podium.yml</code> <code>agents.extra_args</code>.
        </small>
        <select
          aria-label="Argument merge mode"
          value={data.mergeMode}
          onChange={(e) => {
            const mode = e.target.value as MergeMode;
            setData({ ...data, mergeMode: mode });
            agentSettingsSetMergeMode(mode)
              .then(setData)
              .catch((err) =>
                toastError(
                  "Could not save merge mode",
                  toIpcError(err).message,
                ),
              );
          }}
        >
          {MERGE_MODES.map((m) => (
            <option key={m.value} value={m.value}>
              {m.label}
            </option>
          ))}
        </select>
      </section>

      <section className={styles.section}>
        <h3 className={styles.sectionLabel}>Worktrees</h3>
        <SettingToggle
          label="Suggest git worktrees"
          help="When on, agents are asked to offer isolated git-worktree checkouts before modifying code."
          value={data.suggestWorktree}
          onChange={(enabled) => {
            setData({ ...data, suggestWorktree: enabled });
            agentSettingsSetSuggestWorktree(enabled)
              .then(setData)
              .catch((err) =>
                toastError(
                  "Could not save worktree setting",
                  toIpcError(err).message,
                ),
              );
          }}
        />
      </section>

      <section className={styles.section}>
        <h3 className={styles.sectionLabel}>Agents</h3>
        <small className={styles.rowHelp}>
          Set a command override and default arguments applied whenever an agent
          of this type starts.
        </small>
        {data.adapters.length === 0 ? (
          <small className={styles.rowHelp}>No agent adapters found.</small>
        ) : (
          data.adapters.map((adapter) => (
            <AgentCard
              key={adapter.id}
              adapter={adapter}
              onEdit={() => setEditing(adapter.id)}
            />
          ))
        )}
      </section>
    </>
  );
}

/** One adapter card: name, availability, effective command + args, Edit. */
function AgentCard({
  adapter,
  onEdit,
}: {
  adapter: AgentAdapterConfig;
  onEdit: () => void;
}) {
  const command = adapter.command || adapter.binary;
  const preview = [command, ...adapter.defaultArgs].join(" ");
  return (
    <div className={styles.clientCard}>
      <div className={styles.clientHeader}>
        <span className={styles.clientName}>{adapter.displayName}</span>
        <span
          className={styles.clientBadge}
          data-installed={adapter.available ? "true" : "false"}
        >
          <span className={styles.clientBadgeDot} aria-hidden />
          {adapter.available ? "Installed" : "CLI not found"}
        </span>
      </div>
      <div className={styles.clientCommandRow}>
        <code className={styles.clientCommand}>{preview}</code>
        <div className={styles.clientActions}>
          <button type="button" onClick={onEdit}>
            Edit
          </button>
        </div>
      </div>
    </div>
  );
}

/** Inline edit panel for one adapter's command override + default arguments. */
function AgentEditForm({
  adapter,
  onCancel,
  onSaved,
}: {
  adapter: AgentAdapterConfig;
  onCancel: () => void;
  onSaved: (next: AgentSettingsDto) => void;
}) {
  const [command, setCommand] = useState(adapter.command);
  const [argsText, setArgsText] = useState(adapter.defaultArgs.join(" "));
  const [busy, setBusy] = useState(false);

  const save = () => {
    setBusy(true);
    // Split on whitespace; the core trims and drops empties too.
    const args = argsText.split(/\s+/).filter(Boolean);
    agentSettingsSetAdapter(adapter.id, command, args)
      .then((next) => {
        toastSuccess(`${adapter.displayName} settings saved`);
        onSaved(next);
      })
      .catch((e) => {
        toastError("Could not save agent settings", toIpcError(e).message);
        setBusy(false);
      });
  };

  return (
    <form
      className={styles.section}
      onSubmit={(e) => {
        e.preventDefault();
        save();
      }}
    >
      <h3 className={styles.sectionLabel}>Edit {adapter.displayName}</h3>
      <div className={styles.field}>
        <label htmlFor="agent-command">Command</label>
        <input
          id="agent-command"
          type="text"
          value={command}
          placeholder={adapter.binary}
          onChange={(e) => setCommand(e.target.value)}
        />
        <small className={styles.rowHelp}>
          Leave blank to use the default (<code>{adapter.binary}</code>).
        </small>
      </div>
      <div className={styles.field}>
        <label htmlFor="agent-args">Default arguments</label>
        <input
          id="agent-args"
          type="text"
          value={argsText}
          placeholder="--model opus --permission-mode plan"
          onChange={(e) => setArgsText(e.target.value)}
        />
        <small className={styles.rowHelp}>
          Space-separated flags applied when a session starts.
        </small>
      </div>
      <div className={styles.editActions}>
        <button type="button" onClick={onCancel}>
          Cancel
        </button>
        <button type="submit" className="primary" disabled={busy}>
          {busy ? "Saving…" : "Save"}
        </button>
      </div>
    </form>
  );
}

/** Max size for a custom notification sound (kept small — it lives in localStorage). */
const MAX_SOUND_BYTES = 1024 * 1024;

/** Notifications section: sound toggle + optional custom sound file. */
function NotificationsSettings() {
  const settings = useSettingsStore();
  const { sound, soundName } = settings.notifications;
  const fileInput = useRef<HTMLInputElement>(null);

  const onPick = (file: File) => {
    if (file.size > MAX_SOUND_BYTES) {
      toastError("Sound too large", "Pick an audio file under 1 MB.");
      return;
    }
    const reader = new FileReader();
    reader.onload = () => {
      settings.set("notifications", {
        soundDataUrl: String(reader.result),
        soundName: file.name,
      });
      toastSuccess(`Notification sound set: ${file.name}`);
    };
    reader.onerror = () => toastError("Could not read the sound file");
    reader.readAsDataURL(file);
  };

  return (
    <section className={styles.section}>
      <h3 className={styles.sectionLabel}>Notifications</h3>
      <SettingToggle
        label="Play sound"
        help="Play a sound when an agent needs your input."
        value={sound}
        onChange={(v) => settings.set("notifications", { sound: v })}
      />
      <div className={styles.row}>
        <span className={styles.rowLabel}>
          Custom sound
          <small className={styles.rowHelp}>
            {soundName ? soundName : "Built-in beep. Choose your own (< 1 MB)."}
          </small>
        </span>
        <div className={styles.clientActions}>
          <input
            ref={fileInput}
            type="file"
            accept="audio/*"
            hidden
            onChange={(e) => {
              const file = e.target.files?.[0];
              if (file) onPick(file);
              e.target.value = ""; // allow re-picking the same file
            }}
          />
          <button type="button" onClick={() => playNotifySound()}>
            Test
          </button>
          <button type="button" onClick={() => fileInput.current?.click()}>
            Choose…
          </button>
          {soundName && (
            <button
              type="button"
              onClick={() =>
                settings.set("notifications", {
                  soundDataUrl: "",
                  soundName: "",
                })
              }
            >
              Reset
            </button>
          )}
        </div>
      </div>
    </section>
  );
}

function GeneralTab() {
  const mode = useThemeStore((s) => s.mode);
  const setTheme = useThemeStore((s) => s.setTheme);
  const settings = useSettingsStore();
  const fontSize = settings.terminal.fontSize;

  return (
    <>
      {/* Appearance */}
      <section className={styles.section}>
        <h3 className={styles.sectionLabel}>Appearance</h3>
        <div className={styles.themeOptions}>
          {THEMES.map(({ mode: m, label, swatches }) => (
            <button
              key={m}
              type="button"
              className={`${styles.themeOption} ${mode === m ? styles.active : ""}`}
              onClick={() => setTheme(m)}
              aria-pressed={mode === m}
            >
              <span className={styles.swatch}>
                {swatches.map((color, i) => (
                  <span
                    key={i}
                    className={styles.swatchSegment}
                    style={{ background: color }}
                  />
                ))}
              </span>
              <span className={styles.themeLabel}>{label}</span>
              {mode === m && (
                <span className={styles.check} aria-hidden>
                  <CheckIcon />
                </span>
              )}
            </button>
          ))}
        </div>
        <SettingToggle
          label="Reduce motion"
          help="Disable app animations. Your OS reduced-motion preference still applies regardless."
          value={settings.appearance.reduceMotion}
          onChange={(v) => settings.set("appearance", { reduceMotion: v })}
        />
      </section>

      {/* Terminal */}
      <section className={styles.section}>
        <h3 className={styles.sectionLabel}>Terminal</h3>
        <div className={styles.row}>
          <span className={styles.rowLabel}>
            Font size
            <small className={styles.rowHelp}>
              Applies to all terminal views.
            </small>
          </span>
          <div className={styles.stepper}>
            <button
              type="button"
              aria-label="Decrease font size"
              disabled={fontSize <= FONT_SIZE_MIN}
              onClick={() =>
                settings.set("terminal", { fontSize: fontSize - 1 })
              }
            >
              <MinusIcon size={14} />
            </button>
            <span className={styles.stepperValue}>{fontSize} px</span>
            <button
              type="button"
              aria-label="Increase font size"
              disabled={fontSize >= FONT_SIZE_MAX}
              onClick={() =>
                settings.set("terminal", { fontSize: fontSize + 1 })
              }
            >
              <AddIcon size={14} />
            </button>
          </div>
        </div>
        <div className={styles.field}>
          <label htmlFor="terminal-shell">Shell</label>
          <input
            id="terminal-shell"
            type="text"
            value={settings.terminal.shell}
            placeholder={DEFAULT_SHELL_HINT}
            onChange={(e) =>
              settings.set("terminal", { shell: e.target.value })
            }
          />
          <small className={styles.rowHelp}>
            Command new terminals launch. Leave blank for the system default
            (your login shell on macOS/Linux, PowerShell on Windows). Examples:{" "}
            <code>pwsh</code>, <code>cmd</code>, <code>bash</code>,{" "}
            <code>wsl</code>.
          </small>
        </div>
      </section>

      <NotificationsSettings />
    </>
  );
}
