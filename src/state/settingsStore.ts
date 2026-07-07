/** User preferences, persisted to localStorage. Deep-merged over DEFAULTS. */

import { create } from "zustand";

export interface Settings {
  appearance: { reduceMotion: boolean };
  terminal: {
    fontSize: number; // px
  };
}

const DEFAULTS: Settings = {
  appearance: { reduceMotion: false },
  terminal: { fontSize: 13 },
};

const STORAGE_KEY = "podium.settings";

/** Recursively merges `partial` into a fresh deep clone of `base`. Only plain
 *  objects are recursed; arrays/primitives replace. Guards against saved blobs
 *  whose shape predates a newer field. */
function deepMerge<T>(base: T, partial: unknown): T {
  if (
    typeof base !== "object" ||
    base === null ||
    typeof partial !== "object" ||
    partial === null
  ) {
    return (partial as T) ?? base;
  }
  const result = structuredClone(base) as Record<string, unknown>;
  for (const key of Object.keys(partial as object)) {
    const bv = result[key];
    const pv = (partial as Record<string, unknown>)[key];
    result[key] =
      typeof bv === "object" && bv !== null && !Array.isArray(bv)
        ? deepMerge(bv, pv)
        : pv;
  }
  return result as T;
}

function load(): Settings {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return structuredClone(DEFAULTS);
    return deepMerge(DEFAULTS, JSON.parse(raw) as unknown);
  } catch {
    return structuredClone(DEFAULTS);
  }
}

/** Sets the manual reduced-motion override on <html>. Turning the toggle on
 *  sets `data-reduce-motion="true"` (matched by the global.css guard); turning
 *  it off removes the attribute, so the OS-level media query still applies. */
export function applyReduceMotion(reduce: boolean): void {
  const el = document.documentElement;
  if (reduce) {
    el.dataset.reduceMotion = "true";
  } else {
    delete el.dataset.reduceMotion;
  }
}

interface SettingsState extends Settings {
  set: <K extends keyof Settings>(
    section: K,
    patch: Partial<Settings[K]>,
  ) => void;
  resetSettings: () => void;
}

export const useSettingsStore = create<SettingsState>((set, get) => {
  const initial = load();
  applyReduceMotion(initial.appearance.reduceMotion);
  return {
    ...initial,
    set: (section, patch) => {
      const next = { ...get()[section], ...patch } as Settings[typeof section];
      const merged = { ...get(), [section]: next };
      localStorage.setItem(
        STORAGE_KEY,
        JSON.stringify({
          appearance: merged.appearance,
          terminal: merged.terminal,
        }),
      );
      if (section === "appearance") {
        applyReduceMotion((next as Settings["appearance"]).reduceMotion);
      }
      set({ [section]: next } as Partial<SettingsState>);
    },
    resetSettings: () => {
      localStorage.removeItem(STORAGE_KEY);
      applyReduceMotion(DEFAULTS.appearance.reduceMotion);
      set({ ...structuredClone(DEFAULTS) });
    },
  };
});
