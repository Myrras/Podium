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

// Stable per-seed offset so bots do not animate in step.
const delay = (seed: string, mod: number) => {
  let h = 0;
  for (const ch of seed) h = (h * 31 + ch.charCodeAt(0)) % 100003;
  return `-${h % mod}ms`;
};

const frame = (image: string, seed: string, mod: number): CSSProperties => ({
  backgroundImage: image,
  animationDelay: delay(seed, mod),
});

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
      <i style={frame(botSheet(body), seed, 900)} />
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
      <i style={frame(miniSheet(color ?? FAINT), seed, 700)} />
    </span>
  );
}

export function Monitor({ seed }: { seed: string }) {
  return (
    <span className={`${styles.px} ${styles.mon}`} aria-hidden="true">
      <i style={frame(monitorSheet(), seed, 1400)} />
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
