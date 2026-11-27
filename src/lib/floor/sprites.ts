// Pixel art as SVG data URIs (no canvas). Rows and frame patches are copied from the Floor mock.
type Patch = [number, number, string][];

// 16x16 bot at a desk. b body, B shade, f face, e eyes, l laptop lid, g lid glow, d/D desk, a antenna light.
const ROWS = [
  ".......a........",
  ".......A........",
  "....bbbbbbbb....",
  "...bbbbbbbbbb...",
  "...bffffffffb...",
  "...bfeffffefb...",
  "...bffffffffb...",
  "...bbbbbbbbbb...",
  ".....BBBBBB.....",
  "...bbllllllbb...",
  "..bbbllggllbbb..",
  "..B..llllll..B..",
  "dddddddddddddddd",
  "DDDDDDDDDDDDDDDD",
  ".D............D.",
  ".D............D.",
];
const OFF: Patch = [
  [0, 7, "A"],
  [10, 7, "l"],
  [10, 8, "l"],
];
const ARM: Patch = [
  [0, 7, "q"],
  [4, 5, "q"],
  [5, 5, "q"],
  [4, 10, "q"],
  [5, 10, "q"],
  [11, 13, "."],
  [9, 13, "b"],
  [8, 14, "b"],
  [7, 14, "b"],
  [6, 14, "b"],
];
// Frames 0-3 typing, 4-5 raised arm, 6-7 asleep.
const FR: Patch[] = [
  [],
  [
    [11, 2, "."],
    [10, 1, "B"],
    [0, 7, "A"],
  ],
  [
    [5, 5, "f"],
    [5, 6, "e"],
    [5, 10, "f"],
    [5, 11, "e"],
    [11, 13, "."],
    [10, 14, "B"],
  ],
  [
    [10, 7, "l"],
    [10, 8, "l"],
    [0, 7, "A"],
    [11, 2, "."],
    [10, 1, "B"],
  ],
  ARM.concat([[5, 14, "B"]]),
  ARM.concat([
    [5, 14, "b"],
    [4, 14, "B"],
  ]),
  OFF.concat([
    [5, 5, "i"],
    [5, 6, "i"],
    [5, 9, "i"],
    [5, 10, "i"],
  ]),
  OFF.concat([
    [5, 5, "f"],
    [5, 10, "f"],
    [6, 5, "i"],
    [6, 6, "i"],
    [6, 9, "i"],
    [6, 10, "i"],
  ]),
];
const run = (r: number, cs: number[], ch: string): Patch =>
  cs.map((c) => [r, c, ch]);
// Head details per bot design, drawn over every frame (so they win over frame patches). Leave the eye pixels alone so the frames still blink and sleep.
const VARIANTS: Patch[] = [
  // 0 classic: single antenna.
  [],
  // 1 twin antennas.
  [
    [0, 7, "."],
    [1, 7, "."],
    [0, 5, "a"],
    [1, 5, "A"],
    [0, 10, "a"],
    [1, 10, "A"],
  ],
  // 2 side bolts for ears.
  [
    [4, 2, "B"],
    [5, 2, "B"],
    [6, 2, "B"],
    [4, 13, "B"],
    [5, 13, "B"],
    [6, 13, "B"],
  ],
  // 3 visor bar above the eyes.
  run(4, [4, 5, 6, 7, 8, 9, 10, 11], "q"),
  // 4 top hat.
  [
    ...run(0, [5, 6, 7, 8, 9, 10], "A"),
    ...run(1, [5, 6, 7, 8, 9, 10], "q"),
    ...run(2, [3, 4, 5, 6, 7, 8, 9, 10, 11, 12], "A"),
  ],
  // 5 spiky hair tuft.
  [
    [0, 7, "."],
    [0, 6, "A"],
    [0, 8, "A"],
    [0, 10, "A"],
    ...run(1, [5, 6, 7, 8, 9, 10], "A"),
  ],
  // 6 headset: band, ear cups, mic.
  [
    [0, 7, "."],
    ...run(1, [4, 5, 6, 7, 8, 9, 10, 11], "A"),
    [2, 3, "A"],
    [2, 12, "A"],
    [3, 2, "A"],
    [4, 2, "A"],
    [5, 2, "A"],
    [3, 13, "A"],
    [4, 13, "A"],
    [5, 13, "A"],
    [6, 2, "A"],
    [7, 2, "a"],
  ],
  // 7 T-bar antenna and a smile.
  [
    [0, 5, "A"],
    [0, 6, "A"],
    [0, 8, "A"],
    [0, 9, "A"],
    [6, 7, "e"],
    [6, 8, "e"],
  ],
];
export const BOT_VARIANTS = VARIANTS.length;

// 8x8 mini bot (a sub-agent) and 8x8 monitor (a background shell), two frames each; 16x16 monitor on a desk (a terminal).
const MROWS = [
  "...a....",
  ".bbbbbb.",
  ".bffffb.",
  ".beffeb.",
  ".bbbbbb.",
  "..BBBB..",
  ".bbbbbb.",
  "..B..B..",
];
const MFR: Patch[] = [
  [],
  [
    [0, 3, "A"],
    [7, 2, "."],
    [7, 1, "B"],
    [7, 5, "."],
    [7, 6, "B"],
  ],
];
const SROWS = [
  "mmmmmmmm",
  "mggggssm",
  "mssssssm",
  "mgggsssm",
  "mmmmmmmm",
  "...MM...",
  "..MMMM..",
  "........",
];
const SFR: Patch[] = [
  [],
  [
    [1, 3, "s"],
    [1, 4, "s"],
    [3, 4, "g"],
    [3, 5, "g"],
  ],
];
const TROWS = [
  "................",
  "................",
  "...mmmmmmmmmm...",
  "...mssssssssm...",
  "...msggggsssm...",
  "...mssssssssm...",
  "...msggsssssm...",
  "...mssssssssm...",
  "...mmmmmmmmmm...",
  "......MMMM......",
  ".....MMMMMM.....",
  "................",
  "dddddddddddddddd",
  "DDDDDDDDDDDDDDDD",
  ".D............D.",
  ".D............D.",
];

const cache = new Map<string, string>();

// Hex only: SVG data URIs cannot read CSS variables.
export const shade = (hex: string, k: number): string =>
  "#" +
  [1, 3, 5]
    .map((i) =>
      Math.round(parseInt(hex.slice(i, i + 2), 16) * k)
        .toString(16)
        .padStart(2, "0"),
    )
    .join("");

function sheet(
  key: string,
  rows: string[],
  frames: Patch[],
  pal: Record<string, string>,
): string {
  const hit = cache.get(key);
  if (hit) return hit;
  const w = rows[0].length;
  let rects = "";
  frames.forEach((patch, f) => {
    const g = rows.map((r) => r.split(""));
    patch.forEach(([r, c, ch]) => {
      g[r][c] = ch;
    });
    g.forEach((row, r) => {
      for (let c = 0; c < w;) {
        const ch = row[c];
        let n = 1;
        while (c + n < w && row[c + n] === ch) n++;
        if (ch !== ".")
          rects += `<rect x="${f * w + c}" y="${r}" width="${n}" height="1" fill="${pal[ch]}"/>`;
        c += n;
      }
    });
  });
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${w * frames.length}" height="${rows.length}" viewBox="0 0 ${w * frames.length} ${rows.length}" shape-rendering="crispEdges">${rects}</svg>`;
  const url = `url("data:image/svg+xml,${encodeURIComponent(svg)}")`;
  cache.set(key, url);
  return url;
}

const creature = (c: string) => ({
  a: "#56d364",
  A: shade(c, 0.55),
  b: c,
  B: shade(c, 0.72),
  f: "#0d1117",
  e: "#56d364",
});
const SCREEN = { m: "#57606a", M: "#6e7681", s: "#0d1117", g: "#56d364" };
const DESK = { d: "#a67c52", D: "#7b5a3a" };

export function botSheet(color: string, variant = 0): string {
  const v = VARIANTS[variant] ? variant : 0;
  return sheet(
    `b${color}/${v}`,
    ROWS,
    FR.map((p) => p.concat(VARIANTS[v])),
    {
      ...creature(color),
      q: "#79c0ff",
      i: "#6e7681",
      l: "#aeb8c4",
      g: "#ffffff",
      ...DESK,
    },
  );
}
export const miniSheet = (color: string): string =>
  sheet("m" + color, MROWS, MFR, creature(color));
export const monitorSheet = (): string => sheet("mon", SROWS, SFR, SCREEN);
export const terminalDeskSheet = (): string =>
  sheet("term", TROWS, [[], []], { ...SCREEN, ...DESK });
