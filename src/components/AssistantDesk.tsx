/**
 * The assistant's body: the house bot (fixed grey, headset) with its helpers
 * beside it and a one-line state. Reused on the Floor's front desk.
 */

import type { CSSProperties, ReactNode } from "react";

import { useAssistantStore } from "../state/assistantStore";
import { noteSheet } from "../lib/floor/sprites";
import { Bot, MiniBot } from "./floor/Bot";
import styles from "./AssistantDesk.module.css";

export type AssistantDeskState =
  "idle" | "answering" | "reading" | "waiting" | "off";

/** Body colour no agent palette uses. */
export const HOUSE_COLOR = "#c9d1d9";
/** The headset design in `lib/floor/sprites.ts`. */
const HEADSET = 6;
const MAX_HELPERS = 3;

/** Desk state and running-helper count, from the assistant store. */
export function useAssistantDesk(): {
  state: AssistantDeskState;
  helpers: number;
} {
  const enabled = useAssistantStore((s) => s.enabled);
  const busy = useAssistantStore((s) => s.busy);
  const sweeping = useAssistantStore((s) => s.sweeping);
  const waiting = useAssistantStore((s) =>
    s.messages.some((m) => m.proposal?.status === "pending"),
  );
  const helpers = useAssistantStore(
    (s) => s.subagents.filter((a) => a.phase === "running").length,
  );
  const state = !enabled
    ? "off"
    : waiting
      ? "waiting"
      : busy
        ? "answering"
        : sweeping
          ? "reading"
          : "idle";
  return { state, helpers };
}

export function deskLine(state: AssistantDeskState, helpers: number): string {
  switch (state) {
    case "off":
      return "Off";
    case "waiting":
      return "Waiting for your OK";
    case "answering":
      return helpers > 0
        ? `Looking things up with ${helpers} ${helpers === 1 ? "helper" : "helpers"}`
        : "Answering";
    case "reading":
      return "Checking Slack & Teams";
    default:
      return "Idle";
  }
}

const POSE = {
  idle: "still",
  off: "still",
  answering: "typing",
  reading: "typing",
  waiting: "arm",
} as const;

export function AssistantDesk({
  state,
  helpers,
  size = 48,
  hideLine = false,
  children,
}: {
  state: AssistantDeskState;
  helpers: number;
  size?: number;
  /** The host shows the state line itself (the front desk, under its baseline). */
  hideLine?: boolean;
  /** Shown above the state line (the pane's title). */
  children?: ReactNode;
}) {
  const shown = Math.min(helpers, MAX_HELPERS);
  return (
    <span
      className={styles.desk}
      data-state={state}
      data-pose={POSE[state]}
      style={{ "--h": `${size}px` } as CSSProperties}
    >
      {/* "needs" is the bot's raised-arm frame; the CSS swaps it for the
          still frame 0 (eyes open, hands down) when not working. */}
      <Bot
        color={HOUSE_COLOR}
        state={POSE[state] === "typing" ? "working" : "needs"}
        seed="assistant"
        size={size}
        variant={HEADSET}
      />
      {state === "reading" && (
        <span
          className={styles.letters}
          role="img"
          aria-label="Checking Slack and Teams"
          title="Checking Slack and Teams"
        >
          <i style={{ backgroundImage: noteSheet() }} />
          <i style={{ backgroundImage: noteSheet() }} />
        </span>
      )}
      {helpers > 0 && (
        <span className={styles.helpers} title={`${helpers} helpers`}>
          {Array.from({ length: shown }, (_, i) => (
            <span key={i} className={styles.mini}>
              <MiniBot color={HOUSE_COLOR} seed={`assistant-h${i}`} />
            </span>
          ))}
          {helpers > MAX_HELPERS && <b>+{helpers - MAX_HELPERS}</b>}
        </span>
      )}
      {(children || !hideLine) && (
        <span className={styles.text}>
          {children}
          {!hideLine && (
            <span className={styles.line}>{deskLine(state, helpers)}</span>
          )}
        </span>
      )}
    </span>
  );
}
