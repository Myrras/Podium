import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// The stores pull in the IPC layer; jsdom has no Tauri bridge.
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => Promise.resolve([])),
  Channel: class {
    onmessage: (message: unknown) => void = () => undefined;
  },
}));

// Stand in for the real Tiptap editor with a plain textarea — the pane's
// autosave/flush/reconciliation logic only cares about a `content` string
// and an `onChange(markdown)` callback, and driving a real ProseMirror
// contentEditable isn't feasible with `fireEvent`. The real editor's own
// rendering/round-trip/reconciliation behaviour is covered in
// `ScratchpadEditor.test.tsx`.
vi.mock("./ScratchpadEditor", () => ({
  SCRATCHPAD_PLACEHOLDER:
    "Click to type. Notes, research, or handoff details. Markdown supported.",
  ScratchpadEditor: ({
    content,
    onChange,
  }: {
    content: string;
    onChange: (markdown: string) => void;
    onEditorReady?: (editor: unknown) => void;
  }) => (
    <textarea
      aria-label="Scratchpad content"
      placeholder="Click to type. Notes, research, or handoff details. Markdown supported."
      value={content}
      onChange={(e) => onChange(e.target.value)}
    />
  ),
}));

vi.mock("./ScratchpadToolbar", () => ({
  ScratchpadToolbar: () => <div data-testid="scratchpad-toolbar" />,
}));

import type { ScratchpadInfo } from "../ipc/types";
import { useLayoutStore } from "../state/layoutStore";
import { useScratchpadStore } from "../state/scratchpadStore";
import { ScratchpadDetailPane } from "./ScratchpadDetailPane";

const initialScratchpad = useScratchpadStore.getState();
const initialLayout = useLayoutStore.getState();

const PROJECT = "proj-1";
const SCRATCHPAD = "sp-1";

function scratchpad(overrides: Partial<ScratchpadInfo> = {}): ScratchpadInfo {
  return {
    id: SCRATCHPAD,
    projectId: PROJECT,
    title: "Untitled scratchpad",
    content: "",
    archived: false,
    archivedAt: null,
    createdAt: "2024-04-03T12:00:00Z",
    updatedAt: "2024-04-03T12:00:00Z",
    updatedBy: "User",
    version: 1,
    tags: [],
    assignedAgent: null,
    ...overrides,
  };
}

/** Seed the scratchpad store with one scratchpad and spy-able actions. */
function seed(overrides: Partial<ScratchpadInfo> = {}) {
  const updateContent = vi.fn((_projectId, _id, content: string) =>
    Promise.resolve(scratchpad({ ...overrides, content, version: 2 })),
  );
  const updateTitle = vi.fn((_projectId, _id, title: string) =>
    Promise.resolve(scratchpad({ ...overrides, title })),
  );
  const addTag = vi.fn((_projectId, _id, tag: string) =>
    Promise.resolve(scratchpad({ ...overrides, tags: [tag] })),
  );
  const removeTag = vi.fn(() =>
    Promise.resolve(scratchpad({ ...overrides, tags: [] })),
  );
  const setScratchpadArchived = vi.fn(() =>
    Promise.resolve(scratchpad({ ...overrides, archived: true })),
  );
  const refresh = vi.fn(() => Promise.resolve());
  useScratchpadStore.setState(
    {
      ...initialScratchpad,
      scratchpadsByProject: { [PROJECT]: [scratchpad(overrides)] },
      updateContent,
      updateTitle,
      addTag,
      removeTag,
      setScratchpadArchived,
      refresh,
    },
    true,
  );
  return {
    updateContent,
    updateTitle,
    addTag,
    removeTag,
    setScratchpadArchived,
    refresh,
  };
}

describe("ScratchpadDetailPane", () => {
  beforeEach(() => {
    useScratchpadStore.setState(initialScratchpad, true);
    useLayoutStore.setState(initialLayout, true);
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("typing_in_textarea_triggers_debounced_autosave", () => {
    vi.useFakeTimers();
    const { updateContent } = seed();
    render(
      <ScratchpadDetailPane projectId={PROJECT} scratchpadId={SCRATCHPAD} />,
    );

    fireEvent.change(screen.getByLabelText("Scratchpad content"), {
      target: { value: "Some notes" },
    });

    expect(updateContent).not.toHaveBeenCalled();

    vi.advanceTimersByTime(600);

    expect(updateContent).toHaveBeenCalledWith(
      PROJECT,
      SCRATCHPAD,
      "Some notes",
      "2024-04-03T12:00:00Z",
    );
  });

  it("flushes a pending debounced save on unmount instead of dropping it", () => {
    vi.useFakeTimers();
    const { updateContent } = seed();
    const { unmount } = render(
      <ScratchpadDetailPane projectId={PROJECT} scratchpadId={SCRATCHPAD} />,
    );

    // Simulate continuous typing: the debounce timer keeps resetting, so no
    // save has fired yet when the pane closes.
    fireEvent.change(screen.getByLabelText("Scratchpad content"), {
      target: { value: "Some notes tha" },
    });
    vi.advanceTimersByTime(200);
    fireEvent.change(screen.getByLabelText("Scratchpad content"), {
      target: { value: "Some notes that never paused" },
    });

    expect(updateContent).not.toHaveBeenCalled();

    unmount();

    expect(updateContent).toHaveBeenCalledWith(
      PROJECT,
      SCRATCHPAD,
      "Some notes that never paused",
      "2024-04-03T12:00:00Z",
    );
  });

  it("does not flush on unmount when there is no pending save", () => {
    vi.useFakeTimers();
    const { updateContent } = seed();
    const { unmount } = render(
      <ScratchpadDetailPane projectId={PROJECT} scratchpadId={SCRATCHPAD} />,
    );

    unmount();

    expect(updateContent).not.toHaveBeenCalled();
  });

  it("does not flush on unmount once the scratchpad has been removed (e.g. project closed)", () => {
    vi.useFakeTimers();
    const { updateContent } = seed();
    const { unmount } = render(
      <ScratchpadDetailPane projectId={PROJECT} scratchpadId={SCRATCHPAD} />,
    );

    // A pending edit exists...
    fireEvent.change(screen.getByLabelText("Scratchpad content"), {
      target: { value: "unsaved edit" },
    });
    expect(updateContent).not.toHaveBeenCalled();

    // ...but the scratchpad vanishes from the store before the pane unmounts
    // (e.g. the project closed): the pending save must not be flushed, since
    // it would just fail with "not found".
    act(() => {
      useScratchpadStore.setState({
        scratchpadsByProject: { [PROJECT]: [] },
      });
    });
    unmount();

    expect(updateContent).not.toHaveBeenCalled();
  });

  it("footer_shows_author_and_version_from_store", () => {
    seed({ updatedBy: "claude-code", version: 3 });
    render(
      <ScratchpadDetailPane projectId={PROJECT} scratchpadId={SCRATCHPAD} />,
    );

    expect(screen.getByText(/by claude-code/)).toBeInTheDocument();
    expect(screen.getByText(/V3/)).toBeInTheDocument();
  });

  it("renders_placeholder_when_content_empty", () => {
    seed({ content: "" });
    render(
      <ScratchpadDetailPane projectId={PROJECT} scratchpadId={SCRATCHPAD} />,
    );

    expect(
      screen.getByPlaceholderText(
        "Click to type. Notes, research, or handoff details. Markdown supported.",
      ),
    ).toBeInTheDocument();
  });

  it("auto_closes_when_scratchpad_is_removed_from_store", () => {
    const clearOpenScratchpad = vi.fn();
    useScratchpadStore.setState(
      { ...initialScratchpad, scratchpadsByProject: { [PROJECT]: [] } },
      true,
    );
    useLayoutStore.setState({ clearOpenScratchpad });
    const { container } = render(
      <ScratchpadDetailPane projectId={PROJECT} scratchpadId={SCRATCHPAD} />,
    );

    expect(container).toBeEmptyDOMElement();
    expect(clearOpenScratchpad).toHaveBeenCalled();
  });

  it("renders the Tiptap editor and toolbar, not the old plain textarea", () => {
    seed();
    render(
      <ScratchpadDetailPane projectId={PROJECT} scratchpadId={SCRATCHPAD} />,
    );

    expect(screen.getByTestId("scratchpad-toolbar")).toBeInTheDocument();
    expect(screen.getByLabelText("Scratchpad content")).toBeInTheDocument();
  });

  it("toggles fullscreen on and off", () => {
    seed();
    render(
      <ScratchpadDetailPane projectId={PROJECT} scratchpadId={SCRATCHPAD} />,
    );

    fireEvent.click(screen.getByLabelText("Fullscreen"));
    expect(screen.getByLabelText("Exit fullscreen")).toBeInTheDocument();

    fireEvent.click(screen.getByLabelText("Exit fullscreen"));
    expect(screen.getByLabelText("Fullscreen")).toBeInTheDocument();
  });

  it("closes the pane via the close button", () => {
    const clearOpenScratchpad = vi.fn();
    seed();
    useLayoutStore.setState({ clearOpenScratchpad });
    render(
      <ScratchpadDetailPane projectId={PROJECT} scratchpadId={SCRATCHPAD} />,
    );

    fireEvent.click(screen.getByLabelText("Close scratchpad"));
    expect(clearOpenScratchpad).toHaveBeenCalled();
  });

  it("commits a title change on blur", () => {
    const { updateTitle } = seed();
    render(
      <ScratchpadDetailPane projectId={PROJECT} scratchpadId={SCRATCHPAD} />,
    );

    const input = screen.getByLabelText("Scratchpad title");
    fireEvent.change(input, { target: { value: "Renamed" } });
    fireEvent.blur(input);

    expect(updateTitle).toHaveBeenCalledWith(
      PROJECT,
      SCRATCHPAD,
      "Renamed",
      "2024-04-03T12:00:00Z",
    );
  });

  it("archives via the archive button", () => {
    const { setScratchpadArchived } = seed();
    render(
      <ScratchpadDetailPane projectId={PROJECT} scratchpadId={SCRATCHPAD} />,
    );

    fireEvent.click(screen.getByLabelText("Archive scratchpad"));
    expect(setScratchpadArchived).toHaveBeenCalledWith(
      PROJECT,
      SCRATCHPAD,
      true,
    );
  });

  it("renders existing tags and adds a new one", () => {
    const { addTag } = seed({ tags: ["urgent"] });
    render(
      <ScratchpadDetailPane projectId={PROJECT} scratchpadId={SCRATCHPAD} />,
    );

    expect(screen.getByText("urgent")).toBeInTheDocument();

    const input = screen.getByLabelText("Add tag");
    fireEvent.change(input, { target: { value: "backend" } });
    fireEvent.keyDown(input, { key: "Enter" });

    expect(addTag).toHaveBeenCalledWith(PROJECT, SCRATCHPAD, "backend");
  });

  it("blank tag submission is a no-op", () => {
    const { addTag } = seed();
    render(
      <ScratchpadDetailPane projectId={PROJECT} scratchpadId={SCRATCHPAD} />,
    );

    const input = screen.getByLabelText("Add tag");
    fireEvent.change(input, { target: { value: "   " } });
    fireEvent.keyDown(input, { key: "Enter" });

    expect(addTag).not.toHaveBeenCalled();
  });

  it("removes a tag via its chip button", () => {
    const { removeTag } = seed({ tags: ["urgent"] });
    render(
      <ScratchpadDetailPane projectId={PROJECT} scratchpadId={SCRATCHPAD} />,
    );

    fireEvent.click(screen.getByLabelText("Remove tag urgent"));
    expect(removeTag).toHaveBeenCalledWith(PROJECT, SCRATCHPAD, "urgent");
  });

  it("conflict banner freezes the editor on the user's edit until they decide, and reload adopts the server version", async () => {
    vi.useFakeTimers();
    const serverVersion = scratchpad({
      content: "agent's version",
      updatedAt: "2024-04-03T12:05:00Z",
      version: 2,
    });
    const updateContent = vi.fn(() =>
      Promise.resolve({ conflict: true as const }),
    );
    // Simulate the conflict handler's refresh() pulling in the concurrent
    // agent edit — this is the value the adoption effect must NOT swap the
    // editor to while the banner is up.
    const refresh = vi.fn(() => {
      useScratchpadStore.setState({
        scratchpadsByProject: { [PROJECT]: [serverVersion] },
      });
      return Promise.resolve();
    });
    seed();
    useScratchpadStore.setState({ updateContent, refresh });
    render(
      <ScratchpadDetailPane projectId={PROJECT} scratchpadId={SCRATCHPAD} />,
    );

    fireEvent.change(screen.getByLabelText("Scratchpad content"), {
      target: { value: "my stale edit" },
    });
    await act(async () => {
      vi.advanceTimersByTime(600);
      // Let the resolved promise's `.then` run under fake timers.
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(screen.getByRole("alert")).toBeInTheDocument();
    expect(refresh).toHaveBeenCalledWith(PROJECT);
    // The editor still shows the user's edit, not the server's — the banner
    // must not silently discard it before the user decides.
    expect(
      (screen.getByLabelText("Scratchpad content") as HTMLTextAreaElement)
        .value,
    ).toBe("my stale edit");

    fireEvent.click(screen.getByText("Reload"));

    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(
      (screen.getByLabelText("Scratchpad content") as HTMLTextAreaElement)
        .value,
    ).toBe("agent's version");
  });

  it("force save keeps the user's edit visible and persists it with the fresh updatedAt", async () => {
    vi.useFakeTimers();
    const serverVersion = scratchpad({
      content: "agent's version",
      updatedAt: "2024-04-03T12:05:00Z",
      version: 2,
    });
    let calls = 0;
    const updateContent = vi.fn((_p, _id, content: string) => {
      calls += 1;
      if (calls === 1) return Promise.resolve({ conflict: true as const });
      return Promise.resolve(scratchpad({ content, version: 3 }));
    });
    const refresh = vi.fn(() => {
      useScratchpadStore.setState({
        scratchpadsByProject: { [PROJECT]: [serverVersion] },
      });
      return Promise.resolve();
    });
    seed();
    useScratchpadStore.setState({ updateContent, refresh });
    render(
      <ScratchpadDetailPane projectId={PROJECT} scratchpadId={SCRATCHPAD} />,
    );

    fireEvent.change(screen.getByLabelText("Scratchpad content"), {
      target: { value: "my edit" },
    });
    await act(async () => {
      vi.advanceTimersByTime(600);
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(screen.getByRole("alert")).toBeInTheDocument();
    // Still the user's edit, not the server's, while the banner is up.
    expect(
      (screen.getByLabelText("Scratchpad content") as HTMLTextAreaElement)
        .value,
    ).toBe("my edit");

    fireEvent.click(screen.getByText("Force save"));
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });

    // Retried with the fresh (refreshed) updatedAt, not the original stale one.
    expect(updateContent).toHaveBeenLastCalledWith(
      PROJECT,
      SCRATCHPAD,
      "my edit",
      "2024-04-03T12:05:00Z",
    );
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    // The user's edit stays visible — force save doesn't get swapped back to
    // the server's version now that it's the persisted one.
    expect(
      (screen.getByLabelText("Scratchpad content") as HTMLTextAreaElement)
        .value,
    ).toBe("my edit");
  });

  it("force save also persists a pending title edit, not just content", async () => {
    vi.useFakeTimers();
    const serverVersion = scratchpad({
      title: "Agent's title",
      updatedAt: "2024-04-03T12:05:00Z",
      version: 2,
    });
    let titleCalls = 0;
    const updateTitle = vi.fn((_p, _id, title: string) => {
      titleCalls += 1;
      if (titleCalls === 1) return Promise.resolve({ conflict: true as const });
      return Promise.resolve(scratchpad({ title, version: 3 }));
    });
    const updateContent = vi.fn((_p, _id, content: string) =>
      Promise.resolve(scratchpad({ content, version: 3 })),
    );
    const refresh = vi.fn(() => {
      useScratchpadStore.setState({
        scratchpadsByProject: { [PROJECT]: [serverVersion] },
      });
      return Promise.resolve();
    });
    seed();
    useScratchpadStore.setState({ updateTitle, updateContent, refresh });
    render(
      <ScratchpadDetailPane projectId={PROJECT} scratchpadId={SCRATCHPAD} />,
    );

    const titleInput = screen.getByLabelText("Scratchpad title");
    fireEvent.change(titleInput, { target: { value: "My title" } });
    fireEvent.blur(titleInput);
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(screen.getByRole("alert")).toBeInTheDocument();
    // The user's typed title stays visible while the banner is up.
    expect(
      (screen.getByLabelText("Scratchpad title") as HTMLInputElement).value,
    ).toBe("My title");

    fireEvent.click(screen.getByText("Force save"));
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(updateTitle).toHaveBeenCalledWith(
      PROJECT,
      SCRATCHPAD,
      "My title",
      "2024-04-03T12:05:00Z",
    );
    // Content wasn't touched, so it must not have been resaved.
    expect(updateContent).not.toHaveBeenCalled();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    // The user's title stays visible — Force Save must not have been
    // silently dropped in favor of the (never-conflicting) content path.
    expect(
      (screen.getByLabelText("Scratchpad title") as HTMLInputElement).value,
    ).toBe("My title");
  });

  it("typing through a pending conflict keeps re-detecting it instead of silently overwriting the concurrent edit", async () => {
    vi.useFakeTimers();
    const serverVersion = scratchpad({
      content: "agent's version",
      updatedAt: "2024-04-03T12:05:00Z",
      version: 2,
    });
    const updateContent = vi.fn(() =>
      Promise.resolve({ conflict: true as const }),
    );
    const refresh = vi.fn(() => {
      useScratchpadStore.setState({
        scratchpadsByProject: { [PROJECT]: [serverVersion] },
      });
      return Promise.resolve();
    });
    seed();
    useScratchpadStore.setState({ updateContent, refresh });
    render(
      <ScratchpadDetailPane projectId={PROJECT} scratchpadId={SCRATCHPAD} />,
    );

    fireEvent.change(screen.getByLabelText("Scratchpad content"), {
      target: { value: "my stale edit" },
    });
    await act(async () => {
      vi.advanceTimersByTime(600);
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(screen.getByRole("alert")).toBeInTheDocument();
    expect(updateContent).toHaveBeenCalledTimes(1);

    // The user keeps typing instead of clicking Reload/Force Save. If the
    // `expectedUpdatedAt` base had silently advanced to the server's fresh
    // timestamp (pulled in by refresh()), this next autosave would succeed
    // and clobber the concurrent edit without the user ever deciding.
    fireEvent.change(screen.getByLabelText("Scratchpad content"), {
      target: { value: "my stale edit, continued" },
    });
    await act(async () => {
      vi.advanceTimersByTime(600);
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(updateContent).toHaveBeenCalledTimes(2);
    expect(updateContent).toHaveBeenLastCalledWith(
      PROJECT,
      SCRATCHPAD,
      "my stale edit, continued",
      "2024-04-03T12:00:00Z",
    );
    // Still conflicted — the second autosave must not have silently succeeded.
    expect(screen.getByRole("alert")).toBeInTheDocument();
  });
});
