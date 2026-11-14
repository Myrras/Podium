import { BOT_VARIANTS } from "./sprites";

/** Stable 32-bit string hash; also seeds each bot's animation phase. */
export const hash = (id: string) => {
  let h = 0;
  for (const ch of id) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
  return h;
};

/** Bot design per id: its hash slot, or the next free one, in the given order. Unique for up to 8 ids; later ids never move earlier ones. */
export function botVariants(ids: string[]): Map<string, number> {
  const out = new Map<string, number>();
  const used = new Set<number>();
  for (const id of ids) {
    let v = hash(id) % BOT_VARIANTS;
    // All taken (more than 8 ids): the loop wraps back to the hash slot.
    for (let i = 0; i < BOT_VARIANTS && used.has(v); i++)
      v = (v + 1) % BOT_VARIANTS;
    used.add(v);
    out.set(id, v);
  }
  return out;
}
