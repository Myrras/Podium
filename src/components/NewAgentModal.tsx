/**
 * "New agent" dialog: pick an adapter, optionally name the agent and give it
 * an initial prompt, then spawn it (add + start) in the given project.
 */

import { useEffect, useState } from "react";

import { adaptersList, agentSettingsGet, toIpcError } from "../ipc/commands";
import type {
  AdapterInfo,
  AgentAdapterConfig,
  ProjectId,
  TodoId,
} from "../ipc/types";
import { useProcessStore } from "../state/processStore";
import { toastError } from "../state/toastStore";
import { Modal } from "./Modal";
import styles from "./NewAgentModal.module.css";

interface NewAgentModalProps {
  open: boolean;
  projectId: ProjectId | null;
  /** When spawning for to-do(s): their ids seed the agent's prompt. */
  todoIds?: TodoId[];
  /** Prefill the name field (e.g. the to-do's title). */
  initialName?: string;
  onClose: () => void;
}

export function NewAgentModal({
  open,
  projectId,
  todoIds,
  initialName,
  onClose,
}: NewAgentModalProps) {
  const spawnAgent = useProcessStore((s) => s.spawnAgent);

  /** `null` while the adapter list is loading. */
  const [adapters, setAdapters] = useState<AdapterInfo[] | null>(null);
  const [adapterId, setAdapterId] = useState("");
  const [name, setName] = useState("");
  const [prompt, setPrompt] = useState("");
  const [worktree, setWorktree] = useState(false);
  const [busy, setBusy] = useState(false);
  /** Per-adapter default args from settings, to seed the editable args field. */
  const [settingsAdapters, setSettingsAdapters] = useState<
    AgentAdapterConfig[]
  >([]);
  /** CLI args for this session (space-separated), seeded from settings. */
  const [argsText, setArgsText] = useState("");

  // Reset the form and (re)probe adapters on every open.
  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    setAdapters(null);
    setAdapterId("");
    setName(initialName ?? "");
    setPrompt("");
    setWorktree(false);
    setBusy(false);
    setSettingsAdapters([]);
    setArgsText("");
    // Fetch the catalog and the configured default together so the dropdown
    // lands on the user's default agent (Settings → Agents), not just the
    // first available one.
    Promise.all([adaptersList(), agentSettingsGet()])
      .then(([list, settings]) => {
        if (cancelled) return;
        setAdapters(list);
        setSettingsAdapters(settings.adapters);
        const preferred = list.find(
          (a) => a.id === settings.defaultAdapter && a.available,
        );
        setAdapterId(
          (preferred ?? list.find((a) => a.available) ?? list[0])?.id ?? "",
        );
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        setAdapters([]);
        toastError("Could not list agent adapters", toIpcError(e).message);
      });
    return () => {
      cancelled = true;
    };
  }, [open, initialName]);

  // Seed the args field with the selected adapter's default args (Settings →
  // Agents) whenever the adapter changes, so the user starts from the defaults
  // and can tweak them for this session only.
  useEffect(() => {
    const cfg = settingsAdapters.find((a) => a.id === adapterId);
    setArgsText((cfg?.defaultArgs ?? []).join(" "));
  }, [adapterId, settingsAdapters]);

  const selected = adapters?.find((a) => a.id === adapterId) ?? null;
  const canStart = !busy && projectId !== null && selected?.available === true;

  const handleStart = async () => {
    if (!canStart || projectId === null) return;
    setBusy(true);
    const info = await spawnAgent(projectId, {
      adapterId,
      name: name.trim() || undefined,
      prompt: prompt.trim() || undefined,
      todoIds: todoIds && todoIds.length > 0 ? todoIds : undefined,
      worktree: worktree || undefined,
      // Split on whitespace; the core trims and drops empties too.
      args: argsText.split(/\s+/).filter(Boolean),
    });
    setBusy(false);
    if (info) onClose();
  };

  return (
    <Modal
      open={open}
      title={
        todoIds && todoIds.length > 0 ? "Start agent on to-do" : "New agent"
      }
      onClose={onClose}
      footer={
        <>
          <button type="button" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="primary"
            disabled={!canStart}
            onClick={() => void handleStart()}
          >
            {busy ? "Starting…" : "Start agent"}
          </button>
        </>
      }
    >
      <form
        className={styles.form}
        onSubmit={(e) => {
          e.preventDefault();
          void handleStart();
        }}
      >
        <div className={styles.field}>
          <label htmlFor="agent-adapter">Adapter</label>
          <select
            id="agent-adapter"
            value={adapterId}
            disabled={adapters === null}
            onChange={(e) => setAdapterId(e.target.value)}
          >
            {adapters === null && <option value="">Loading…</option>}
            {adapters?.map((a) => (
              <option key={a.id} value={a.id}>
                {a.displayName}
                {a.available ? "" : " (not installed)"}
              </option>
            ))}
          </select>
          {selected && !selected.available && (
            <p className={styles.warning}>
              The {selected.displayName} CLI was not found on your PATH.
            </p>
          )}
        </div>

        <div className={styles.field}>
          <label htmlFor="agent-name">Name</label>
          <input
            id="agent-name"
            type="text"
            value={name}
            placeholder="Automatic (claude, claude-2, …)"
            onChange={(e) => setName(e.target.value)}
          />
        </div>

        <div className={styles.field}>
          <label htmlFor="agent-prompt">Initial prompt (optional)</label>
          <textarea
            id="agent-prompt"
            className={styles.prompt}
            value={prompt}
            rows={3}
            placeholder="What should the agent work on?"
            onChange={(e) => setPrompt(e.target.value)}
          />
        </div>

        <div className={styles.field}>
          <label htmlFor="agent-args">Arguments</label>
          <input
            id="agent-args"
            type="text"
            value={argsText}
            placeholder="--model opus --permission-mode plan"
            onChange={(e) => setArgsText(e.target.value)}
          />
          <p className={styles.help}>
            Space-separated CLI flags for this session (seeded from Settings →
            Agents).
          </p>
        </div>

        <div className={styles.field}>
          <label className={styles.checkboxLabel}>
            <input
              type="checkbox"
              checked={worktree}
              onChange={(e) => setWorktree(e.target.checked)}
            />
            Run in a git worktree
          </label>
          <p className={styles.help}>
            Creates an isolated checkout under .podium/worktrees so the agent's
            changes don't touch your working tree.
          </p>
        </div>

        {/* Hidden submit so Enter in the inputs starts the agent. Inline
            display:none because the global `button` rule overrides `hidden`. */}
        <button
          type="submit"
          style={{ display: "none" }}
          disabled={!canStart}
        />
      </form>
    </Modal>
  );
}
