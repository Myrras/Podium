import type { CSSProperties } from "react";
import {
  botSheet,
  miniSheet,
  monitorSheet,
  terminalDeskSheet,
} from "../../lib/floor/sprites";
import type { DeskState } from "../../lib/floor/types";
import styles from "./Bot.module.css";

// Hex stand-ins: a data-URI sprite cannot use CSS variables (--text-faint, grey for stopped bots).
const FAINT = "#6e7681";
const GONE = "#8b949e";

// Stable per-seed phase (0..1) so bots do not animate in step; CSS turns it into a delay.
const phase = (seed: string) => {
  let h = 0;
  for (const ch of seed) h = (h * 31 + ch.charCodeAt(0)) % 100003;
  return h / 100003;
};

const frame = (image: string, seed: string): CSSProperties =>
  ({ backgroundImage: image, "--phase": phase(seed) }) as CSSProperties;

export function Bot({
  color,
  state,
  seed,
  size,
}: {
  color: string | null;
  state: DeskState;
  seed: string;
  size?: number;
}) {
  const body =
    state === "ended" || state === "crashed" ? GONE : (color ?? FAINT);
  return (
    <span
      className={styles.bot}
      data-state={state}
      style={size ? ({ "--s": `${size}px` } as CSSProperties) : undefined}
      aria-hidden="true"
    >
      <i style={frame(botSheet(body), seed)} />
    </span>
  );
}

export function MiniBot({
  color,
  seed,
}: {
  color: string | null;
  seed: string;
}) {
  return (
    <span className={styles.px} aria-hidden="true">
      <i style={frame(miniSheet(color ?? FAINT), seed)} />
    </span>
  );
}

export function Monitor({ seed }: { seed: string }) {
  return (
    <span className={`${styles.px} ${styles.mon}`} aria-hidden="true">
      <i style={frame(monitorSheet(), seed)} />
    </span>
  );
}

export function TerminalDeskArt() {
  return (
    <span className={`${styles.bot} ${styles.two}`} aria-hidden="true">
      <i style={{ backgroundImage: terminalDeskSheet() }} />
    </span>
  );
}
