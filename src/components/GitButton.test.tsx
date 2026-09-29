import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../ipc/commands", () => ({
  issuesBoard: vi.fn(() => new Promise(() => {})),
  issuesSettingsGet: vi.fn(() => new Promise(() => {})),
  toIpcError: (e: unknown) => e,
}));

import { issuesBoard } from "../ipc/commands";
import type { BoardItem } from "../ipc/types";
import { useIssueStore } from "../state/issueStore";
import { useLayoutStore } from "../state/layoutStore";
import { useProjectStore } from "../state/projectStore";
import { GitButton } from "./GitButton";

const P = "proj-1";

const card = (column: BoardItem["column"], number: number): BoardItem => ({
  kind: "issue",
  number,
  title: `Item ${number}`,
  url: `https://github.com/acme/widgets/issues/${number}`,
  body: "",
  labels: [],
  updatedAt: "2026-09-01T10:00:00Z",
  column,
  draft: false,
  author: "",
});

beforeEach(() => {
  vi.mocked(issuesBoard).mockClear();
  useIssueStore.setState({ byProject: {}, overview: null }, false);
  useLayoutStore.setState({ openBoard: null });
  useProjectStore.setState({ activeProjectId: null });
});

describe("GitButton", () => {
  it("fetches on mount and opens the board on click", () => {
    render(<GitButton projectId={P} />);
    expect(issuesBoard).toHaveBeenCalledWith(P, true);
    fireEvent.click(screen.getByRole("button", { name: "Git board" }));
    expect(useLayoutStore.getState().openBoard).toEqual({ projectId: P });
    expect(useProjectStore.getState().activeProjectId).toBe(P);
    expect(screen.getByRole("button").getAttribute("data-open")).toBe("true");
  });

  it("summarises the board, or flags an error", () => {
    useIssueStore.setState({
      byProject: {
        [P]: {
          board: {
            provider: "github",
            prCounts: { open: 1, merged: 1 },
            items: [card("open", 1), card("inProgress", 2), card("review", 3)],
          },
          error: null,
          loading: true,
          fetchedAt: 1,
          mine: true,
        },
      },
    });
    const { rerender } = render(<GitButton projectId={P} />);
    expect(
      screen.getByRole("button", { name: "Git board — 2 open · 1 review" }),
    ).toBeTruthy();
    useIssueStore.setState({
      byProject: {
        [P]: {
          board: null,
          error: { kind: "issuesNoRemote", message: "x" },
          loading: false,
          fetchedAt: 1,
          mine: true,
        },
      },
    });
    rerender(<GitButton projectId={P} />);
    const btn = screen.getByRole("button");
    expect(btn.getAttribute("aria-label")).toBe(
      "Git board — No GitHub or GitLab remote found.",
    );
    expect(btn.getAttribute("data-error")).toBe("true");
  });
});
