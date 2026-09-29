import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../ipc/commands", () => ({
  issuesBoard: vi.fn(),
  issuesBoardCached: vi.fn(),
  issuesBoardMove: vi.fn(),
  toIpcError: (e: unknown) => e as { kind: string; message: string },
}));

vi.mock("./toastStore", () => ({ toastError: vi.fn() }));

import {
  issuesBoard,
  issuesBoardCached,
  issuesBoardMove,
} from "../ipc/commands";
import type { Board } from "../ipc/types";
import { toastError } from "./toastStore";
import {
  ISSUE_FOCUS_STALE_MS,
  ISSUE_POLL_MS,
  useIssueStore,
} from "./issueStore";

const listMock = vi.mocked(issuesBoard);
const cachedMock = vi.mocked(issuesBoardCached);
const moveMock = vi.mocked(issuesBoardMove);
const P = "proj-1";

function board(...numbers: number[]): Board {
  return {
    provider: "github",
    prCounts: { open: 1, merged: 2 },
    columns: [],
    source: null,
    hint: null,
    me: null,
    items: numbers.map((number) => ({
      kind: "issue",
      number,
      title: `Issue ${number}`,
      url: `https://github.com/acme/widgets/issues/${number}`,
      body: "",
      labels: [],
      updatedAt: "2026-09-01T10:00:00Z",
      column: "open",
      draft: false,
      author: "",
      movable: true,
      linked: [],
      assignees: [],
      approvals: null,
    })),
  };
}

beforeEach(() => {
  vi.useFakeTimers();
  listMock.mockReset();
  cachedMock.mockReset().mockResolvedValue(null);
  moveMock.mockReset();
  vi.mocked(toastError).mockClear();
  useIssueStore.setState({ byProject: {} }, false);
});
afterEach(() => vi.useRealTimers());

describe("issueStore", () => {
  it("dedupes a refresh while one is in flight", async () => {
    let resolve!: (v: Board) => void;
    listMock.mockReturnValue(new Promise((r) => (resolve = r)));
    const a = useIssueStore.getState().refresh(P);
    const b = useIssueStore.getState().refresh(P);
    expect(listMock).toHaveBeenCalledTimes(1);
    expect(useIssueStore.getState().byProject[P].loading).toBe(true);
    resolve(board(1));
    await Promise.all([a, b]);
    expect(useIssueStore.getState().byProject[P]).toMatchObject({
      board: board(1),
      loading: false,
      error: null,
    });
  });

  it("keeps the stale list on error and clears the error on success", async () => {
    listMock.mockResolvedValueOnce(board(1));
    await useIssueStore.getState().refresh(P);
    listMock.mockRejectedValueOnce({ kind: "issuesFetch", message: "boom" });
    await useIssueStore.getState().refresh(P);
    expect(useIssueStore.getState().byProject[P]).toMatchObject({
      board: board(1),
      error: { kind: "issuesFetch" },
      loading: false,
    });
    listMock.mockResolvedValueOnce(board());
    await useIssueStore.getState().refresh(P);
    expect(useIssueStore.getState().byProject[P]).toMatchObject({
      board: board(),
      error: null,
    });
  });

  it("auto-refresh: fetches on start, polls, and stops", async () => {
    listMock.mockResolvedValue(board());
    const stop = useIssueStore.getState().startAutoRefresh(P);
    await vi.advanceTimersByTimeAsync(0);
    expect(listMock).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(ISSUE_POLL_MS);
    expect(listMock).toHaveBeenCalledTimes(2);
    stop();
    await vi.advanceTimersByTimeAsync(ISSUE_POLL_MS * 2);
    expect(listMock).toHaveBeenCalledTimes(2);
  });

  it("auto-refresh: refreshes on focus only past the staleness threshold", async () => {
    listMock.mockResolvedValue(board());
    const stop = useIssueStore.getState().startAutoRefresh(P);
    await vi.advanceTimersByTimeAsync(0);
    expect(listMock).toHaveBeenCalledTimes(1);

    await vi.advanceTimersByTimeAsync(ISSUE_FOCUS_STALE_MS - 1000);
    window.dispatchEvent(new Event("focus"));
    expect(listMock).toHaveBeenCalledTimes(1);

    await vi.advanceTimersByTimeAsync(2000);
    window.dispatchEvent(new Event("focus"));
    expect(listMock).toHaveBeenCalledTimes(2);

    stop();
    await vi.advanceTimersByTimeAsync(ISSUE_FOCUS_STALE_MS + 1000);
    window.dispatchEvent(new Event("focus"));
    expect(listMock).toHaveBeenCalledTimes(2);
  });

  it("fetches the user's own items by default", async () => {
    listMock.mockResolvedValue(board());
    await useIssueStore.getState().refresh(P);
    expect(listMock).toHaveBeenCalledWith(P, true);
  });

  it("setMine refetches, and a switch mid-fetch refetches for the new scope", async () => {
    let resolve!: (v: Board) => void;
    listMock.mockReturnValueOnce(new Promise((r) => (resolve = r)));
    listMock.mockResolvedValue(board(2));
    const first = useIssueStore.getState().refresh(P);
    useIssueStore.getState().setMine(P, false);
    // The switch waits for the fetch in flight rather than racing it.
    expect(listMock).toHaveBeenCalledTimes(1);
    resolve(board(1));
    await first;
    expect(listMock).toHaveBeenLastCalledWith(P, false);
    expect(useIssueStore.getState().byProject[P]).toMatchObject({
      board: board(2),
      mine: false,
    });
  });

  it("auto-refresh: a fresh cache is shown without an immediate fetch", async () => {
    cachedMock.mockResolvedValue({
      board: board(3),
      fetchedAt: new Date(Date.now() - 1000).toISOString(),
    });
    listMock.mockResolvedValue(board(4));
    const stop = useIssueStore.getState().startAutoRefresh(P);
    await vi.advanceTimersByTimeAsync(0);
    expect(cachedMock).toHaveBeenCalledWith(P, true);
    expect(listMock).not.toHaveBeenCalled();
    expect(useIssueStore.getState().byProject[P].board).toEqual(board(3));
    expect(useIssueStore.getState().byProject[P].fetchedAt).toBeGreaterThan(0);
    stop();
  });

  it("auto-refresh: a stale cache is shown, then refetched", async () => {
    cachedMock.mockResolvedValue({
      board: board(3),
      fetchedAt: new Date(Date.now() - ISSUE_POLL_MS - 1000).toISOString(),
    });
    listMock.mockResolvedValue(board(4));
    const stop = useIssueStore.getState().startAutoRefresh(P);
    await vi.advanceTimersByTimeAsync(0);
    expect(listMock).toHaveBeenCalledTimes(1);
    expect(useIssueStore.getState().byProject[P].board).toEqual(board(4));
    stop();
  });

  it("setMine shows that scope's cache before the refetch lands", async () => {
    cachedMock.mockResolvedValue({
      board: board(5),
      fetchedAt: new Date().toISOString(),
    });
    listMock.mockReturnValue(new Promise(() => {}));
    useIssueStore.getState().setMine(P, false);
    await vi.advanceTimersByTimeAsync(0);
    expect(cachedMock).toHaveBeenCalledWith(P, false);
    expect(useIssueStore.getState().byProject[P].board).toEqual(board(5));
    expect(listMock).toHaveBeenCalledWith(P, false);
  });

  it("hydrate re-reads the cache (board:changed)", async () => {
    cachedMock.mockResolvedValue({
      board: board(6),
      fetchedAt: new Date().toISOString(),
    });
    await useIssueStore.getState().hydrate(P);
    expect(useIssueStore.getState().byProject[P].board).toEqual(board(6));
  });

  it("move patches the card at once and keeps it when the backend accepts", async () => {
    listMock.mockResolvedValue(board(1));
    await useIssueStore.getState().refresh(P);
    let resolve!: () => void;
    moveMock.mockReturnValue(new Promise((r) => (resolve = r)));
    const p = useIssueStore.getState().move(P, board(1).items[0], "inProgress");
    expect(useIssueStore.getState().byProject[P].board?.items[0].column).toBe(
      "inProgress",
    );
    expect(moveMock).toHaveBeenCalledWith(P, "issue", 1, "inProgress");
    resolve();
    await p;
    expect(useIssueStore.getState().byProject[P].board?.items[0].column).toBe(
      "inProgress",
    );
    expect(toastError).not.toHaveBeenCalled();
  });

  it("move reverts and toasts when the backend refuses", async () => {
    listMock.mockResolvedValue(board(1));
    await useIssueStore.getState().refresh(P);
    moveMock.mockRejectedValue({ kind: "issuesMove", message: "Not allowed" });
    await useIssueStore.getState().move(P, board(1).items[0], "done");
    expect(useIssueStore.getState().byProject[P].board?.items[0].column).toBe(
      "open",
    );
    expect(toastError).toHaveBeenCalledWith("Not allowed");
    expect(cachedMock).toHaveBeenCalled();
  });

  it("dropProject forgets the cache", async () => {
    listMock.mockResolvedValue(board(1));
    await useIssueStore.getState().refresh(P);
    useIssueStore.getState().dropProject(P);
    expect(useIssueStore.getState().byProject[P]).toBeUndefined();
  });
});
