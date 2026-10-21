/**
 * Parse git's unified diff into rows the changes panel renders: hunk
 * headers plus context/added/removed lines with their old/new line numbers.
 * The file header (`diff --git`, `index`, `---`/`+++`, mode lines) before
 * the first hunk is dropped — the panel shows the path itself.
 */

export type DiffRowKind = "hunk" | "context" | "add" | "del" | "meta";

export interface DiffRow {
  kind: DiffRowKind;
  text: string;
  /** Line number in the old file (context/del rows). */
  oldLine: number | null;
  /** Line number in the new file (context/add rows). */
  newLine: number | null;
}

const HUNK = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/;

export function parseUnifiedDiff(diff: string): DiffRow[] {
  const rows: DiffRow[] = [];
  let oldLine = 0;
  let newLine = 0;
  let inHunk = false;
  for (const raw of diff.split("\n")) {
    // Git keeps a file's CRLF endings in the diff; the \r is noise here.
    const line = raw.endsWith("\r") ? raw.slice(0, -1) : raw;
    const hunk = HUNK.exec(line);
    if (hunk) {
      inHunk = true;
      oldLine = Number(hunk[1]);
      newLine = Number(hunk[2]);
      rows.push({ kind: "hunk", text: line, oldLine: null, newLine: null });
      continue;
    }
    if (!inHunk) continue;
    if (line.startsWith("diff --git ")) {
      // A second file section (should not happen for one path): stop at it.
      inHunk = false;
      continue;
    }
    const sign = line[0];
    const text = line.slice(1);
    if (sign === "+") {
      rows.push({ kind: "add", text, oldLine: null, newLine: newLine++ });
    } else if (sign === "-") {
      rows.push({ kind: "del", text, oldLine: oldLine++, newLine: null });
    } else if (sign === " ") {
      rows.push({
        kind: "context",
        text,
        oldLine: oldLine++,
        newLine: newLine++,
      });
    } else if (sign === "\\") {
      // "\ No newline at end of file"
      rows.push({ kind: "meta", text: line, oldLine: null, newLine: null });
    }
  }
  return rows;
}
