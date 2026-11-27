/** First-run chooser: Project or Floor mode. Shows while no mode is stored. */

import { useState } from "react";

import { useLayoutStore } from "../state/layoutStore";
import { Modal } from "./Modal";
import styles from "./ModeChooser.module.css";

const CHOICES = [
  ["project", "Project mode", "The sidebar with your projects, as before."],
  [
    "floor",
    "Floor mode",
    "A map with a room per project and a desk per agent.",
  ],
] as const;

export function ModeChooser() {
  const mode = useLayoutStore((s) => s.mode);
  const setMode = useLayoutStore((s) => s.setMode);
  // Closing without a choice: not asked again in this run, `mode` stays null.
  const [dismissed, setDismissed] = useState(false);

  return (
    <Modal
      open={mode === null && !dismissed}
      title="Choose your view"
      onClose={() => setDismissed(true)}
    >
      <div className={styles.choices}>
        {CHOICES.map(([m, label, line]) => (
          <button
            key={m}
            type="button"
            className={styles.choice}
            onClick={() => setMode(m)}
          >
            <span className={styles.label}>{label}</span>
            <span className={styles.line}>{line}</span>
          </button>
        ))}
      </div>
    </Modal>
  );
}
