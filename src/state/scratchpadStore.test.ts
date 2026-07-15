import { beforeEach, describe, expect, it, vi } from "vitest";

// Mock the IPC command layer so the store talks to fixtures, not Tauri.
vi.mock("../ipc/commands", () => ({
  scratchpadList: vi.fn(() => Promise.resolve([])),
  scratchpadListArchived: vi.fn(() => Promise.resolve([])),
  scratchpadAdd: vi.fn(),
  scratchpadUpdateContent: vi.fn(),
  scratchpadUpdateTitle: vi.fn(),
  scratchpadAddTag: vi.fn(),
  scratchpadRemoveTag: vi.fn(),
  scratchpadSetArchived: vi.fn(),
  toIpcError: (e: unknown) => {
    if (typeof e === "object" && e !== null && "kind" in e && "message" in e) {
      return e as { kind: string; message: string };
    }
    return {
      kind: "io",
      message: e instanceof Error ? e.message : String(e),
    };
  },
}));

// Swallow toasts (no DOM assertions here).
vi.mock("./toastStore", () => ({ toastError: vi.fn() }));

import type { ScratchpadInfo } from "../ipc/types";
import {
  scratchpadAdd,
  scratchpadAddTag,
  scratchpadList,
  scratchpadListArchived,
  scratchpadRemoveTag,
  scratchpadSetArchived,
  scratchpadUpdateContent,
  scratchpadUpdateTitle,
} from "../ipc/commands";
import { toastError } from "./toastStore";
import { useScratchpadStore } from "./scratchpadStore";

const scratchpadListMock = vi.mocked(scratchpadList);
const scratchpadListArchivedMock = vi.mocked(scratchpadListArchived);
const scratchpadAddMock = vi.mocked(scratchpadAdd);
const scratchpadUpdateContentMock = vi.mocked(scratchpadUpdateContent);
const scratchpadUpdateTitleMock = vi.mocked(scratchpadUpdateTitle);
const scratchpadAddTagMock = vi.mocked(scratchpadAddTag);
const scratchpadRemoveTagMock = vi.mocked(scratchpadRemoveTag);
const scratchpadSetArchivedMock = vi.mocked(scratchpadSetArchived);
const toastErrorMock = vi.mocked(toastError);

/** A fictitious scratchpad snapshot for store fixtures. */
function scratchpad(
  id: string,
  overrides: Partial<ScratchpadInfo> = {},
): ScratchpadInfo {
  return {
    id,
    projectId: "proj-a",
    title: "07-14-09-30 Scratchpad",
    content: "",
    archived: false,
    archivedAt: null,
    createdAt: "2026-07-14T09:30:00Z",
    updatedAt: "2026-07-14T09:30:00Z",
    updatedBy: "User",
    version: 1,
    tags: [],
    assignedAgent: null,
    ...overrides,
  };
}

describe("scratchpadStore", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useScratchpadStore.setState(
      { scratchpadsByProject: {}, archivedByProject: {} },
      false,
    );
  });

  it("refresh_populates_scratchpads_by_project", async () => {
    scratchpadListMock.mockResolvedValue([scratchpad("sp-1")]);

    await useScratchpadStore.getState().refresh("proj-a");

    expect(scratchpadListMock).toHaveBeenCalledWith("proj-a");
    expect(
      useScratchpadStore.getState().scratchpadsByProject["proj-a"],
    ).toEqual([scratchpad("sp-1")]);
  });

  it("refresh_archived_populates_archived_by_project", async () => {
    const archived = scratchpad("sp-1", {
      archived: true,
      archivedAt: "2026-07-14T10:00:00Z",
    });
    scratchpadListArchivedMock.mockResolvedValue([archived]);

    await useScratchpadStore.getState().refreshArchived("proj-a");

    expect(scratchpadListArchivedMock).toHaveBeenCalledWith("proj-a");
    expect(useScratchpadStore.getState().archivedByProject["proj-a"]).toEqual([
      archived,
    ]);
  });

  it("add_scratchpad_eagerly_updates_state", async () => {
    scratchpadAddMock.mockResolvedValue(scratchpad("sp-1"));

    const result = await useScratchpadStore.getState().addScratchpad("proj-a");

    expect(scratchpadAddMock).toHaveBeenCalledWith("proj-a");
    expect(result).toEqual(scratchpad("sp-1"));
    expect(
      useScratchpadStore.getState().scratchpadsByProject["proj-a"],
    ).toEqual([scratchpad("sp-1")]);
  });

  it("update_content_eagerly_updates_state_and_echoes_updated_at_verbatim", async () => {
    useScratchpadStore.setState(
      { scratchpadsByProject: { "proj-a": [scratchpad("sp-1")] } },
      false,
    );
    const updated = scratchpad("sp-1", {
      content: "hello",
      version: 2,
      updatedAt: "2026-07-14T09:31:00.123456789Z",
    });
    scratchpadUpdateContentMock.mockResolvedValue(updated);

    const result = await useScratchpadStore
      .getState()
      .updateContent(
        "proj-a",
        "sp-1",
        "hello",
        "2026-07-14T09:30:00.987654321Z",
      );

    // The exact opaque string is passed through untouched — no parsing or
    // reformatting, since the backend compares it for exact equality.
    expect(scratchpadUpdateContentMock).toHaveBeenCalledWith(
      "proj-a",
      "sp-1",
      "hello",
      "2026-07-14T09:30:00.987654321Z",
    );
    expect(result).toEqual(updated);
    expect(
      useScratchpadStore.getState().scratchpadsByProject["proj-a"],
    ).toEqual([updated]);
  });

  it("update_content_conflict_returns_marker_without_toasting", async () => {
    useScratchpadStore.setState(
      { scratchpadsByProject: { "proj-a": [scratchpad("sp-1")] } },
      false,
    );
    scratchpadUpdateContentMock.mockRejectedValue({
      kind: "scratchpadConflict",
      message: "scratchpad conflict",
    });

    const result = await useScratchpadStore
      .getState()
      .updateContent("proj-a", "sp-1", "stale edit", "2026-07-14T09:30:00Z");

    expect(result).toEqual({ conflict: true });
    expect(toastErrorMock).not.toHaveBeenCalled();
    // The stale local snapshot is left untouched — the caller decides.
    expect(
      useScratchpadStore.getState().scratchpadsByProject["proj-a"],
    ).toEqual([scratchpad("sp-1")]);
  });

  it("update_content_other_failure_toasts_and_returns_null", async () => {
    scratchpadUpdateContentMock.mockRejectedValue(new Error("disk full"));

    const result = await useScratchpadStore
      .getState()
      .updateContent("proj-a", "sp-1", "hello", "2026-07-14T09:30:00Z");

    expect(result).toBeNull();
    expect(toastErrorMock).toHaveBeenCalledWith(
      "Could not update scratchpad",
      "disk full",
    );
  });

  it("update_title_eagerly_updates_state", async () => {
    useScratchpadStore.setState(
      { scratchpadsByProject: { "proj-a": [scratchpad("sp-1")] } },
      false,
    );
    const updated = scratchpad("sp-1", { title: "My Notes" });
    scratchpadUpdateTitleMock.mockResolvedValue(updated);

    const result = await useScratchpadStore
      .getState()
      .updateTitle("proj-a", "sp-1", "My Notes", "2026-07-14T09:30:00Z");

    expect(scratchpadUpdateTitleMock).toHaveBeenCalledWith(
      "proj-a",
      "sp-1",
      "My Notes",
      "2026-07-14T09:30:00Z",
    );
    expect(result).toEqual(updated);
    expect(
      useScratchpadStore.getState().scratchpadsByProject["proj-a"],
    ).toEqual([updated]);
  });

  it("update_title_conflict_returns_marker_without_toasting", async () => {
    scratchpadUpdateTitleMock.mockRejectedValue({
      kind: "scratchpadConflict",
      message: "scratchpad conflict",
    });

    const result = await useScratchpadStore
      .getState()
      .updateTitle("proj-a", "sp-1", "Renamed", "2026-07-14T09:30:00Z");

    expect(result).toEqual({ conflict: true });
    expect(toastErrorMock).not.toHaveBeenCalled();
  });

  it("add_tag_eagerly_updates_state", async () => {
    useScratchpadStore.setState(
      { scratchpadsByProject: { "proj-a": [scratchpad("sp-1")] } },
      false,
    );
    const tagged = scratchpad("sp-1", { tags: ["urgent"] });
    scratchpadAddTagMock.mockResolvedValue(tagged);

    const result = await useScratchpadStore
      .getState()
      .addTag("proj-a", "sp-1", "urgent");

    expect(scratchpadAddTagMock).toHaveBeenCalledWith(
      "proj-a",
      "sp-1",
      "urgent",
    );
    expect(result).toEqual(tagged);
    expect(
      useScratchpadStore.getState().scratchpadsByProject["proj-a"],
    ).toEqual([tagged]);
  });

  it("remove_tag_eagerly_updates_state", async () => {
    useScratchpadStore.setState(
      {
        scratchpadsByProject: {
          "proj-a": [scratchpad("sp-1", { tags: ["urgent"] })],
        },
      },
      false,
    );
    const untagged = scratchpad("sp-1", { tags: [] });
    scratchpadRemoveTagMock.mockResolvedValue(untagged);

    const result = await useScratchpadStore
      .getState()
      .removeTag("proj-a", "sp-1", "urgent");

    expect(scratchpadRemoveTagMock).toHaveBeenCalledWith(
      "proj-a",
      "sp-1",
      "urgent",
    );
    expect(result).toEqual(untagged);
    expect(
      useScratchpadStore.getState().scratchpadsByProject["proj-a"],
    ).toEqual([untagged]);
  });

  it("set_scratchpad_archived_moves_between_active_and_archived_caches", async () => {
    useScratchpadStore.setState(
      {
        scratchpadsByProject: { "proj-a": [scratchpad("sp-1")] },
        archivedByProject: {},
      },
      false,
    );
    const archived = scratchpad("sp-1", {
      archived: true,
      archivedAt: "2026-07-14T10:00:00Z",
    });
    scratchpadSetArchivedMock.mockResolvedValue(archived);

    await useScratchpadStore
      .getState()
      .setScratchpadArchived("proj-a", "sp-1", true);

    expect(scratchpadSetArchivedMock).toHaveBeenCalledWith(
      "proj-a",
      "sp-1",
      true,
    );
    expect(
      useScratchpadStore.getState().scratchpadsByProject["proj-a"],
    ).toEqual([]);
    expect(useScratchpadStore.getState().archivedByProject["proj-a"]).toEqual([
      archived,
    ]);

    const restored = scratchpad("sp-1");
    scratchpadSetArchivedMock.mockResolvedValue(restored);

    await useScratchpadStore
      .getState()
      .setScratchpadArchived("proj-a", "sp-1", false);

    expect(
      useScratchpadStore.getState().scratchpadsByProject["proj-a"],
    ).toEqual([restored]);
    expect(useScratchpadStore.getState().archivedByProject["proj-a"]).toEqual(
      [],
    );
  });

  it("refresh_handles_command_failure_via_toast_error", async () => {
    scratchpadListMock.mockRejectedValue(new Error("disk full"));

    await useScratchpadStore.getState().refresh("proj-a");

    expect(toastErrorMock).toHaveBeenCalledWith(
      "Failed to list scratchpads",
      "disk full",
    );
    expect(
      useScratchpadStore.getState().scratchpadsByProject["proj-a"],
    ).toBeUndefined();
  });

  it("drop_project_removes_the_projects_cached_lists", () => {
    useScratchpadStore.setState(
      {
        scratchpadsByProject: { "proj-a": [scratchpad("sp-1")] },
        archivedByProject: { "proj-a": [scratchpad("sp-2")] },
      },
      false,
    );

    useScratchpadStore.getState().dropProject("proj-a");

    expect(
      useScratchpadStore.getState().scratchpadsByProject["proj-a"],
    ).toBeUndefined();
    expect(
      useScratchpadStore.getState().archivedByProject["proj-a"],
    ).toBeUndefined();
  });
});
