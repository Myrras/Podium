import { beforeEach, describe, expect, it, vi } from "vitest";

// Mock the IPC command layer so the store talks to fixtures, not Tauri.
vi.mock("../ipc/commands", () => ({
  projectOpen: vi.fn(),
  projectList: vi.fn(() => Promise.resolve([])),
  projectRename: vi.fn(),
  projectReorder: vi.fn(),
  recentsList: vi.fn(() => Promise.resolve([])),
  workspaceList: vi.fn(),
  workspaceRemove: vi.fn(() => Promise.resolve([])),
  toIpcError: (e: unknown) => ({
    kind: "io",
    message: e instanceof Error ? e.message : String(e),
  }),
}));

// Swallow toasts (no DOM assertions here); expose just enough of the toast
// store for the restore-failure toast's remove action to dismiss itself.
const { requestDismissMock } = vi.hoisted(() => ({
  requestDismissMock: vi.fn(),
}));
vi.mock("./toastStore", () => ({
  toastError: vi.fn(() => 1),
  useToastStore: { getState: () => ({ requestDismiss: requestDismissMock }) },
}));

import type { ProjectInfo } from "../ipc/types";
import {
  projectList,
  projectOpen,
  projectRename,
  projectReorder,
  workspaceList,
  workspaceRemove,
} from "../ipc/commands";
import { toastError } from "./toastStore";
import { reorderIds, useProjectStore } from "./projectStore";

const projectOpenMock = vi.mocked(projectOpen);
const projectListMock = vi.mocked(projectList);
const projectRenameMock = vi.mocked(projectRename);
const projectReorderMock = vi.mocked(projectReorder);
const workspaceListMock = vi.mocked(workspaceList);
const workspaceRemoveMock = vi.mocked(workspaceRemove);
const toastErrorMock = vi.mocked(toastError);

/** A fictitious project snapshot for store fixtures. */
function project(id: string, name = id): ProjectInfo {
  return {
    id,
    name,
    root: `/projects/${id}`,
    iconInitials: "",
    configError: null,
    renamed: false,
  };
}

describe("projectStore.restoreWorkspace", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useProjectStore.setState(
      { projects: [], activeProjectId: null, recents: [] },
      false,
    );
  });

  it("keeps a failed entry in the workspace instead of pruning it", async () => {
    // "Once opened it stays open": a transient open failure (e.g. an
    // external drive not yet mounted) must not drop the project.
    workspaceListMock.mockResolvedValue(["/projects/webshop"]);
    projectOpenMock.mockRejectedValue(new Error("not a directory"));
    projectListMock.mockResolvedValue([]);

    await useProjectStore.getState().restoreWorkspace();

    expect(projectOpenMock).toHaveBeenCalledWith("/projects/webshop");
    expect(workspaceRemoveMock).not.toHaveBeenCalled();
  });

  it("offers a remove-from-workspace action on a failed restore", async () => {
    // A permanent failure (folder moved/deleted) would otherwise fail on
    // every launch forever; the toast's action lets the user drop it.
    workspaceListMock.mockResolvedValue(["/projects/gone"]);
    projectOpenMock.mockRejectedValue(new Error("not a directory"));
    projectListMock.mockResolvedValue([]);

    await useProjectStore.getState().restoreWorkspace();

    expect(toastErrorMock).toHaveBeenCalledTimes(1);
    const opts = toastErrorMock.mock.calls[0][2];
    expect(opts?.sticky).toBe(true);
    expect(opts?.action?.label).toBe("Remove from workspace");

    await opts?.action?.onClick();

    expect(workspaceRemoveMock).toHaveBeenCalledWith("/projects/gone");
    expect(requestDismissMock).toHaveBeenCalledWith(1);
  });

  it("restores every persisted project in order", async () => {
    workspaceListMock.mockResolvedValue(["/a", "/b"]);
    projectOpenMock.mockImplementation((path) =>
      Promise.resolve({
        id: path,
        name: path,
        root: path,
        iconInitials: "",
        configError: null,
        renamed: false,
      }),
    );
    projectListMock.mockResolvedValue([]);

    await useProjectStore.getState().restoreWorkspace();

    expect(projectOpenMock).toHaveBeenNthCalledWith(1, "/a");
    expect(projectOpenMock).toHaveBeenNthCalledWith(2, "/b");
    expect(workspaceRemoveMock).not.toHaveBeenCalled();
  });
});

describe("reorderIds", () => {
  it("moves an id before the target", () => {
    expect(reorderIds(["a", "b", "c"], "c", "a")).toEqual(["c", "a", "b"]);
    expect(reorderIds(["a", "b", "c"], "a", "c")).toEqual(["b", "a", "c"]);
  });

  it("appends when the target is null or unknown", () => {
    expect(reorderIds(["a", "b", "c"], "a", null)).toEqual(["b", "c", "a"]);
    expect(reorderIds(["a", "b", "c"], "a", "zzz")).toEqual(["b", "c", "a"]);
  });

  it("is a no-op when moved before itself", () => {
    expect(reorderIds(["a", "b", "c"], "b", "b")).toEqual(["a", "c", "b"]);
  });
});

describe("projectStore.renameProject", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useProjectStore.setState(
      { projects: [project("a")], activeProjectId: null, recents: [] },
      false,
    );
  });

  it("replaces the project with the renamed snapshot", async () => {
    projectRenameMock.mockResolvedValue({
      ...project("a"),
      name: "Alpha",
      renamed: true,
    });

    await useProjectStore.getState().renameProject("a", "Alpha");

    expect(projectRenameMock).toHaveBeenCalledWith("a", "Alpha");
    const [p] = useProjectStore.getState().projects;
    expect(p.name).toBe("Alpha");
    expect(p.renamed).toBe(true);
  });
});

describe("projectStore.reorderProjects", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useProjectStore.setState(
      {
        projects: [project("a"), project("b"), project("c")],
        activeProjectId: null,
        recents: [],
      },
      false,
    );
  });

  it("optimistically reorders then adopts the backend order", async () => {
    projectReorderMock.mockResolvedValue([
      project("c"),
      project("a"),
      project("b"),
    ]);

    await useProjectStore.getState().reorderProjects("c", "a");

    expect(projectReorderMock).toHaveBeenCalledWith(["c", "a", "b"]);
    const ids = useProjectStore.getState().projects.map((p) => p.id);
    expect(ids).toEqual(["c", "a", "b"]);
  });

  it("rolls back to the previous order when persisting fails", async () => {
    projectReorderMock.mockRejectedValue(new Error("disk full"));

    await useProjectStore.getState().reorderProjects("c", "a");

    const ids = useProjectStore.getState().projects.map((p) => p.id);
    expect(ids).toEqual(["a", "b", "c"]);
  });
});
