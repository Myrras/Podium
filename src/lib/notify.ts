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

/** Alert the user that an agent has stalled awaiting permission or input. */
export function notifyAgentWaiting(agentName: string): void {
  const title = "Agent needs your input";
  const body = `${agentName} is waiting for permission or input.`;
  toastInfo(title, body);
  void ensureNotificationPermission().then((granted) => {
    if (!granted) return;
    try {
      sendNotification({ title, body });
    } catch (e) {
      logWarning("notification", (e as Error).message ?? String(e));
    }
  });
}
