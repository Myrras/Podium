/**
 * User-facing alerts for background agent events.
 *
 * "Agent needs your input" fires both a native OS notification (so it's seen
 * even when Podium isn't focused) and an in-app toast (an on-screen trace).
 * The OS notification is best-effort: permission is requested lazily the
 * first time we actually need it, and any failure is logged, never surfaced.
 * Text here is Podium-owned — no terminal output or secrets.
 */

import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";

import { toastInfo } from "../state/toastStore";
import { useSettingsStore } from "../state/settingsStore";
import { logWarning } from "./log";

/** Cached permission result: null = not yet checked. */
let permissionGranted: boolean | null = null;

/** Resolve (and cache) whether OS notifications may be shown, requesting once. */
async function ensureNotificationPermission(): Promise<boolean> {
  if (permissionGranted !== null) return permissionGranted;
  try {
    let granted = await isPermissionGranted();
    if (!granted) granted = (await requestPermission()) === "granted";
    permissionGranted = granted;
  } catch (e) {
    logWarning("notification permission", (e as Error).message ?? String(e));
    permissionGranted = false;
  }
  return permissionGranted;
}

/** Synthesize a short two-tone "ding" — the built-in default, no asset needed. */
function playDefaultBeep(): void {
  try {
    const ctx = new AudioContext();
    const now = ctx.currentTime;
    const gain = ctx.createGain();
    gain.connect(ctx.destination);
    // Quick attack, gentle decay; two overlapping tones for a notification feel.
    gain.gain.setValueAtTime(0.0001, now);
    gain.gain.exponentialRampToValueAtTime(0.25, now + 0.01);
    gain.gain.exponentialRampToValueAtTime(0.0001, now + 0.35);
    for (const [freq, at] of [
      [880, now],
      [1320, now + 0.09],
    ] as const) {
      const osc = ctx.createOscillator();
      osc.type = "sine";
      osc.frequency.value = freq;
      osc.connect(gain);
      osc.start(at);
      osc.stop(now + 0.35);
    }
    // Release the context once the sound has finished.
    window.setTimeout(() => void ctx.close(), 500);
  } catch (e) {
    logWarning("notification sound", (e as Error).message ?? String(e));
  }
}

/** Play the notification sound if enabled: a user's custom file, else the beep. */
export function playNotifySound(): void {
  const { sound, soundDataUrl } = useSettingsStore.getState().notifications;
  if (!sound) return;
  if (!soundDataUrl) {
    playDefaultBeep();
    return;
  }
  try {
    void new Audio(soundDataUrl).play();
  } catch (e) {
    logWarning("notification sound", (e as Error).message ?? String(e));
  }
}

/** Alert the user that an agent has stalled awaiting permission or input. */
export function notifyAgentWaiting(agentName: string): void {
  const title = "Agent needs your input";
  const body = `${agentName} is waiting for permission or input.`;
  toastInfo(title, body);
  playNotifySound();
  void ensureNotificationPermission().then((granted) => {
    if (!granted) return;
    try {
      sendNotification({ title, body });
    } catch (e) {
      logWarning("notification", (e as Error).message ?? String(e));
    }
  });
}
