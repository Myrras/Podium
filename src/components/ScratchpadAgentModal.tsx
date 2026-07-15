/**
 * "Start agent on scratchpad" dialog: pick an adapter, optionally name the
 * agent and give it an initial prompt, then spawn it (add + start) in the
 * given project working on one or more scratchpads.
 *
 * A sibling of `NewAgentModal` rather than a reuse of it — scratchpads always
 * open this modal to confirm/add instructions before spawning (unlike to-dos,
 * there is no plain-click direct-spawn shortcut), since a scratchpad's
 * content can be long-lived context worth reviewing first.
 */

import { useEffect, useState } from "react";

import { adaptersList, agentSettingsGet, toIpcError } from "../ipc/commands";
import type { AdapterInfo, ProjectId, ScratchpadId } from "../ipc/types";
import { useProcessStore } from "../state/processStore";
import { toastError } from "../state/toastStore";
import { Modal } from "./Modal";
import styles from "./NewAgentModal.module.css";

interface ScratchpadAgentModalProps {
  open: boolean;
  projectId: ProjectId | null;
  /** The scratchpad(s) to work on; seeds the agent's prompt. */
  scratchpadIds: ScratchpadId[];
  /** Prefill the name field (e.g. the first scratchpad's title). */
  initialName?: string;
  onClose: () => void;
}

export function ScratchpadAgentModal({
  open,
  projectId,
  scratchpadIds,
  initialName,
  onClose,
}: ScratchpadAgentModalProps) {
  const spawnAgent = useProcessStore((s) => s.spawnAgent);

  /** `null` while the adapter list is loading. */
  const [adapters, setAdapters] = useState<AdapterInfo[] | null>(null);
  const [adapterId, setAdapterId] = useState("");
  const [name, setName] = useState("");
  const [prompt, setPrompt] = useState("");
  const [busy, setBusy] = useState(false);

  // Reset the form and (re)probe adapters on every open.
  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    setAdapters(null);
    setAdapterId("");
    setName(initialName ?? "");
    setPrompt("");
    setBusy(false);
    // Fetch the catalog and the configured default together so the dropdown
    // lands on the user's default agent (Settings → Agents), not just the
    // first available one.
    Promise.all([adaptersList(), agentSettingsGet()])
      .then(([list, settings]) => {
        if (cancelled) return;
        setAdapters(list);
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

  const selected = adapters?.find((a) => a.id === adapterId) ?? null;
  const canStart = !busy && projectId !== null && selected?.available === true;

  const handleStart = async () => {
    if (!canStart || projectId === null) return;
    setBusy(true);
    const info = await spawnAgent(projectId, {
      adapterId,
      name: name.trim() || undefined,
      prompt: prompt.trim() || undefined,
      scratchpadIds: scratchpadIds.length > 0 ? scratchpadIds : undefined,
    });
    setBusy(false);
    if (info) onClose();
  };

  return (
    <Modal
      open={open}
      title={
        scratchpadIds.length > 1
          ? `Start agent on ${scratchpadIds.length} scratchpads`
          : "Start agent on scratchpad"
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
          <label htmlFor="scratchpad-agent-adapter">Adapter</label>
          <select
            id="scratchpad-agent-adapter"
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
          <label htmlFor="scratchpad-agent-name">Name</label>
          <input
            id="scratchpad-agent-name"
            type="text"
            value={name}
            placeholder="Automatic (claude, claude-2, …)"
            onChange={(e) => setName(e.target.value)}
          />
        </div>

        <div className={styles.field}>
          <label htmlFor="scratchpad-agent-prompt">
            Additional instructions (optional)
          </label>
          <textarea
            id="scratchpad-agent-prompt"
            className={styles.prompt}
            value={prompt}
            rows={3}
            placeholder="What should the agent do with this scratchpad?"
            onChange={(e) => setPrompt(e.target.value)}
          />
        </div>

        {/* Hidden submit so Enter in the inputs starts the agent. */}
        <button type="submit" hidden disabled={!canStart} />
      </form>
    </Modal>
  );
}
