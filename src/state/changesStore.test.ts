import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../ipc/commands", () => ({
  changesList: vi.fn(),
  changesFileDiff: vi.fn(),
  toIpcError: (e: unknown) => e,
}));

import type { ChangeSet, ChangedFile } from "../ipc/types";
import { changesFileDiff, changesList } from "../ipc/commands";
import { useChangesStore } from "./changesStore";

const listMock = vi.mocked(changesList);
const diffMock = vi.mocked(changesFileDiff);
const initial = useChangesStore.getState();

const ROOT = { projectId: "proj-1", worktree: null };
const WT = { projectId: "proj-1", worktree: "fix-cart" };

function file(path: string): ChangedFile {
  return {
    path,
    oldPath: null,
    status: "modified",
    additions: 1,
    deletions: 0,
    binary: false,
  };
}

function set(...paths: string[]): ChangeSet {
  return {
    branch: "main",
    base: "uncommitted",
    baseRef: null,
    baseCommit: "abc12345",
    files: paths.map(file),
    truncated: false,
  };
}

function deferred<T>() {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => (resolve = r));
  return { promise, resolve };
}

describe("changesStore", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useChangesStore.setState(initial, true);
    diffMock.mockResolvedValue({
      path: "a.ts",
      diff: "@@ -1 +1 @@\n-a\n+b\n",
      binary: false,
      truncated: false,
    });
  });

  it("drops an answer for a target that is no longer current", async () => {
    const slow = deferred<ChangeSet>();
    listMock.mockReturnValueOnce(slow.promise);
    listMock.mockResolvedValueOnce(set("worktree.ts"));

    const first = useChangesStore.getState().load(ROOT);
    await useChangesStore.getState().load(WT);
    slow.resolve(set("root.ts"));
    await first;

    expect(useChangesStore.getState().changes?.files[0].path).toBe(
      "worktree.ts",
    );
  });

  it("re-fetches expanded diffs on refresh and forgets files that are gone", async () => {
    listMock.mockResolvedValue(set("a.ts", "b.ts"));
    await useChangesStore.getState().load(ROOT);
    useChangesStore.getState().toggleFile(ROOT, file("a.ts"));
    useChangesStore.getState().toggleFile(ROOT, file("b.ts"));
    expect(diffMock).toHaveBeenCalledTimes(2);

    listMock.mockResolvedValue(set("a.ts"));
    await useChangesStore.getState().load(ROOT);

    expect(useChangesStore.getState().expanded).toEqual({ "a.ts": true });
    expect(diffMock).toHaveBeenCalledTimes(3);
    expect(diffMock).toHaveBeenLastCalledWith(
      "proj-1",
      null,
      "uncommitted",
      "a.ts",
      null,
    );
  });

  it("clears expansions when the target changes", async () => {
    listMock.mockResolvedValue(set("a.ts"));
    await useChangesStore.getState().load(ROOT);
    useChangesStore.getState().toggleFile(ROOT, file("a.ts"));

    await useChangesStore.getState().load(WT);

    expect(useChangesStore.getState().expanded).toEqual({});
    expect(useChangesStore.getState().diffs).toEqual({});
  });

  it("collapsing a file does not fetch", async () => {
    listMock.mockResolvedValue(set("a.ts"));
    await useChangesStore.getState().load(ROOT);
    useChangesStore.getState().toggleFile(ROOT, file("a.ts"));
    useChangesStore.getState().toggleFile(ROOT, file("a.ts"));
    expect(diffMock).toHaveBeenCalledTimes(1);
    expect(useChangesStore.getState().expanded["a.ts"]).toBe(false);
  });
});
