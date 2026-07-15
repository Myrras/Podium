import { render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// The stores pull in the IPC layer; jsdom has no Tauri bridge.
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => Promise.resolve([])),
  Channel: class {
    onmessage: (message: unknown) => void = () => undefined;
  },
}));

// Unlike `ScratchpadDetailPane.test.tsx`, this file exercises the real
// `ScratchpadEditor`/Tiptap instance (not the textarea stand-in) — the TOC
// panel reads real heading nodes off `editor.state.doc`, so it needs a real
// document, not a plain string prop.

import type { ScratchpadInfo } from "../ipc/types";
import { useLayoutStore } from "../state/layoutStore";
import { useScratchpadStore } from "../state/scratchpadStore";
import { ScratchpadDetailPane } from "./ScratchpadDetailPane";

const initialScratchpad = useScratchpadStore.getState();
const initialLayout = useLayoutStore.getState();

const PROJECT = "proj-1";
const SCRATCHPAD_A = "sp-a";
const SCRATCHPAD_B = "sp-b";

function scratchpad(overrides: Partial<ScratchpadInfo> = {}): ScratchpadInfo {
  return {
    id: SCRATCHPAD_A,
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

function seed(scratchpads: ScratchpadInfo[]) {
  useScratchpadStore.setState(
    {
      ...initialScratchpad,
      scratchpadsByProject: { [PROJECT]: scratchpads },
      updateContent: vi.fn((_projectId, id, content: string) =>
        Promise.resolve(
          scratchpad({ id, content, version: 2 }) as ScratchpadInfo,
        ),
      ),
      refresh: vi.fn(() => Promise.resolve()),
    },
    true,
  );
}

describe("ScratchpadDetailPane — TOC panel", () => {
  beforeEach(() => {
    useScratchpadStore.setState(initialScratchpad, true);
    useLayoutStore.setState(initialLayout, true);
  });

  it("renders the TOC panel alongside the editor, reflecting the open scratchpad's headings", async () => {
    seed([
      scratchpad({
        id: SCRATCHPAD_A,
        content: "## First section\n\nSome text.\n\n### Nested subsection",
      }),
    ]);

    render(
      <ScratchpadDetailPane
        key={SCRATCHPAD_A}
        projectId={PROJECT}
        scratchpadId={SCRATCHPAD_A}
      />,
    );

    // The editor and the TOC panel both render at once, as siblings.
    expect(screen.getByLabelText("Scratchpad content")).toBeInTheDocument();
    const toc = screen.getByRole("navigation", { name: "On this page" });
    expect(toc).toBeInTheDocument();

    await waitFor(() => {
      expect(within(toc).getByText("First section")).toBeInTheDocument();
      expect(within(toc).getByText("Nested subsection")).toBeInTheDocument();
    });
  });

  it("shows the empty state when the open scratchpad has no headings", async () => {
    seed([scratchpad({ id: SCRATCHPAD_A, content: "Just a paragraph." })]);

    render(
      <ScratchpadDetailPane
        key={SCRATCHPAD_A}
        projectId={PROJECT}
        scratchpadId={SCRATCHPAD_A}
      />,
    );

    await waitFor(() => {
      expect(screen.getByText("No headings yet")).toBeInTheDocument();
    });
  });

  it("updates the TOC when switching between scratchpads", async () => {
    seed([
      scratchpad({ id: SCRATCHPAD_A, content: "## Alpha heading" }),
      scratchpad({
        id: SCRATCHPAD_B,
        projectId: PROJECT,
        content: "## Beta heading\n\n### Beta sub",
      }),
    ]);

    const { rerender } = render(
      <ScratchpadDetailPane
        key={SCRATCHPAD_A}
        projectId={PROJECT}
        scratchpadId={SCRATCHPAD_A}
      />,
    );

    let toc = screen.getByRole("navigation", { name: "On this page" });
    await waitFor(() => {
      expect(within(toc).getByText("Alpha heading")).toBeInTheDocument();
    });
    expect(within(toc).queryByText("Beta heading")).not.toBeInTheDocument();

    // App.tsx keys ScratchpadDetailPane by scratchpad id, so switching
    // scratchpads remounts the pane (and its editor) from scratch — mirror
    // that here via `key` on rerender rather than just changing props.
    rerender(
      <ScratchpadDetailPane
        key={SCRATCHPAD_B}
        projectId={PROJECT}
        scratchpadId={SCRATCHPAD_B}
      />,
    );

    toc = screen.getByRole("navigation", { name: "On this page" });
    await waitFor(() => {
      expect(within(toc).getByText("Beta heading")).toBeInTheDocument();
      expect(within(toc).getByText("Beta sub")).toBeInTheDocument();
    });
    expect(within(toc).queryByText("Alpha heading")).not.toBeInTheDocument();
  });

  it("clicking a TOC entry scrolls the editor to that heading without changing content", async () => {
    seed([
      scratchpad({
        id: SCRATCHPAD_A,
        content: "## First\n\nSome text.\n\n## Second\n\nMore text.",
      }),
    ]);

    render(
      <ScratchpadDetailPane
        key={SCRATCHPAD_A}
        projectId={PROJECT}
        scratchpadId={SCRATCHPAD_A}
      />,
    );

    const toc = screen.getByRole("navigation", { name: "On this page" });
    await waitFor(() => {
      expect(within(toc).getByText("Second")).toBeInTheDocument();
    });

    const editor = screen.getByLabelText("Scratchpad content");
    const before = editor.textContent;

    within(toc).getByText("Second").click();

    await waitFor(() => {
      expect(editor.textContent).toBe(before);
    });
  });
});
