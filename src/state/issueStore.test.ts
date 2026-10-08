import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../ipc/commands", () => ({
  issuesBoard: vi.fn(),
  issuesBoardCached: vi.fn(),
  issuesBoardMove: vi.fn(),
  issuesSettingsGet: vi.fn(),
  toIpcError: (e: unknown) => e as { kind: string; message: string },
}));

vi.mock("./toastStore", () => ({ toastError: vi.fn() }));

import {
  issuesBoard,
  issuesBoardCached,
  issuesBoardMove,
  issuesSettingsGet,
} from "../ipc/commands";
import type { Board, IssuesOverview } from "../ipc/types";
import { toastError } from "./toastStore";
import {
  ISSUE_FOCUS_STALE_MS,
  AUTO_FETCH_SPACING_MS,
  ISSUE_POLL_MS,
  moveTargets,
  resetAutoFetchPacing,
  useIssueStore,
} from "./issueStore";

const listMock = vi.mocked(issuesBoard);
const cachedMock = vi.mocked(issuesBoardCached);
const moveMock = vi.mocked(issuesBoardMove);
const settingsMock = vi.mocked(issuesSettingsGet);
const P = "proj-1";

const CONFIGURED: IssuesOverview = {
  setups: [
    {
      id: "s1",
      name: "GitHub",
      provider: "github",
      url: "https://github.com",
      tokenSet: false,
    },
  ],
  projects: {},
};
const UNCONFIGURED: IssuesOverview = { setups: [], projects: {} };

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
  settingsMock.mockReset();
  resetAutoFetchPacing();
  useIssueStore.setState({ byProject: {}, overview: CONFIGURED }, false);
});
afterEach(() => vi.useRealTimers());

describe("issueStore", () => {
  it("never asks the backend for a board while no setup exists", async () => {
    useIssueStore.setState({ overview: UNCONFIGURED });
    await useIssueStore.getState().refresh(P);
    expect(listMock).not.toHaveBeenCalled();
    expect(useIssueStore.getState().byProject[P]).toMatchObject({
      board: null,
      loading: false,
      error: { kind: "issuesNotConfigured" },
    });
  });

  it("loads the setups first when they are not known yet", async () => {
    useIssueStore.setState({ overview: null });
    settingsMock.mockResolvedValue(UNCONFIGURED);
    await useIssueStore.getState().refresh(P);
    expect(settingsMock).toHaveBeenCalledTimes(1);
    expect(listMock).not.toHaveBeenCalled();

    // Adding a setup refetches the shown project, now for real.
    listMock.mockResolvedValue(board(3));
    useIssueStore.getState().setOverview(CONFIGURED);
    await vi.waitFor(() =>
      expect(useIssueStore.getState().byProject[P].board).toEqual(board(3)),
    );
  });

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

  it("auto-refresh: projects fetch one pacing slot apart, a manual refresh at once", async () => {
    listMock.mockResolvedValue(board(1));
    const stops = ["a", "b", "c"].map((p) =>
      useIssueStore.getState().startAutoRefresh(p),
    );
    await vi.advanceTimersByTimeAsync(0);
    expect(listMock).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(AUTO_FETCH_SPACING_MS);
    expect(listMock).toHaveBeenCalledTimes(2);
    await useIssueStore.getState().refresh("d");
    expect(listMock).toHaveBeenCalledTimes(3);
    await vi.advanceTimersByTimeAsync(AUTO_FETCH_SPACING_MS);
    expect(listMock.mock.calls.map(([p]) => p)).toEqual(["a", "b", "d", "c"]);
    stops.forEach((stop) => stop());
  });

  it("auto-refresh: a queued project is not loading, and a manual refresh runs at once", async () => {
    listMock.mockResolvedValue(board(1));
    const stops = ["a", "b"].map((p) =>
      useIssueStore.getState().startAutoRefresh(p),
    );
    await vi.advanceTimersByTimeAsync(0);
    expect(listMock.mock.calls.map(([p]) => p)).toEqual(["a"]);
    expect(useIssueStore.getState().byProject.b?.loading ?? false).toBe(false);
    await useIssueStore.getState().refresh("b");
    expect(listMock.mock.calls.map(([p]) => p)).toEqual(["a", "b"]);
    // The queued auto refresh of "b" lapses instead of fetching again.
    await vi.advanceTimersByTimeAsync(AUTO_FETCH_SPACING_MS * 2);
    expect(listMock.mock.calls.map(([p]) => p)).toEqual(["a", "b"]);
    stops.forEach((stop) => stop());
  });

  it("auto-refresh: a project closed while queued is not brought back", async () => {
    listMock.mockResolvedValue(board(1));
    const stops = ["a", "b"].map((p) =>
      useIssueStore.getState().startAutoRefresh(p),
    );
    await vi.advanceTimersByTimeAsync(0);
    useIssueStore.getState().dropProject("b");
    await vi.advanceTimersByTimeAsync(AUTO_FETCH_SPACING_MS * 2);
    expect(listMock.mock.calls.map(([p]) => p)).toEqual(["a"]);
    expect(useIssueStore.getState().byProject.b).toBeUndefined();
    stops.forEach((stop) => stop());
  });

  it("auto-refresh: closing during hydrate gives no fetch and no entry", async () => {
    let answer!: (v: { board: Board; fetchedAt: string }) => void;
    cachedMock.mockReturnValueOnce(new Promise((r) => (answer = r)));
    listMock.mockResolvedValue(board(1));
    const stop = useIssueStore.getState().startAutoRefresh(P);
    useIssueStore.getState().dropProject(P);
    stop();
    // A stale cache: it would be shown, then refetched, were it still open.
    answer({ board: board(3), fetchedAt: "2026-01-01T00:00:00Z" });
    await vi.advanceTimersByTimeAsync(AUTO_FETCH_SPACING_MS);
    expect(listMock).not.toHaveBeenCalled();
    expect(useIssueStore.getState().byProject[P]).toBeUndefined();
  });

  it("auto-refresh: a project stuck on a local error takes no slot", async () => {
    listMock.mockResolvedValue(board(1));
    useIssueStore.setState({
      byProject: {
        a: {
          board: null,
          error: { kind: "issuesNoRemote", message: "no remote" },
          loading: false,
          fetchedAt: 0,
          mine: true,
        },
      },
    });
    const stops = ["a", "b"].map((p) =>
      useIssueStore.getState().startAutoRefresh(p),
    );
    await vi.advanceTimersByTimeAsync(0);
    // Both go at once: "a" did not use up the free slot.
    expect(listMock.mock.calls.map(([p]) => p).sort()).toEqual(["a", "b"]);
    stops.forEach((stop) => stop());
  });

  it("a setup change mid-fetch refetches once that fetch is done", async () => {
    let resolve!: (v: Board) => void;
    listMock.mockReturnValueOnce(new Promise((r) => (resolve = r)));
    listMock.mockResolvedValue(board(2));
    const first = useIssueStore.getState().refresh(P);
    useIssueStore.getState().setOverview(CONFIGURED);
    expect(listMock).toHaveBeenCalledTimes(1);
    resolve(board(1));
    await first;
    expect(listMock).toHaveBeenCalledTimes(2);
    expect(useIssueStore.getState().byProject[P].board).toEqual(board(2));
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

  it("a Mine answer landing after a switch to All never shows on All", async () => {
    let resolveMine!: (v: Board) => void;
    let resolveAll!: (v: Board) => void;
    listMock
      .mockReturnValueOnce(new Promise((r) => (resolveMine = r)))
      .mockReturnValueOnce(new Promise((r) => (resolveAll = r)));
    cachedMock.mockResolvedValue({
      board: board(7),
      fetchedAt: new Date().toISOString(),
    });
    const first = useIssueStore.getState().refresh(P);
    useIssueStore.getState().setMine(P, false);
    await vi.advanceTimersByTimeAsync(0);
    expect(useIssueStore.getState().byProject[P].board).toEqual(board(7));
    resolveMine(board(1));
    await vi.advanceTimersByTimeAsync(0);
    // The All board (its cache) stays; the Mine answer is dropped.
    expect(useIssueStore.getState().byProject[P].board).toEqual(board(7));
    expect(listMock).toHaveBeenLastCalledWith(P, false);
    resolveAll(board(2));
    await first;
    expect(useIssueStore.getState().byProject[P].board).toEqual(board(2));
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

  it("moveTargets: without project columns Open is only reachable from Done", () => {
    const columns = ["open", "label:doing", "done"].map((id) => ({
      id,
      title: id,
      color: null,
      accepts: ["issue" as const],
    }));
    const ids = (column: string, source: string | null) =>
      moveTargets({ ...board(1).items[0], column }, columns, source).map(
        (c) => c.id,
      );
    expect(ids("label:doing", null)).toEqual(["done"]);
    expect(ids("done", null)).toEqual(["open", "label:doing"]);
    expect(ids("label:doing", "Dev")).toEqual(["open", "done"]);
  });

  it("dropProject forgets the cache", async () => {
    listMock.mockResolvedValue(board(1));
    await useIssueStore.getState().refresh(P);
    useIssueStore.getState().dropProject(P);
    expect(useIssueStore.getState().byProject[P]).toBeUndefined();
  });
});
