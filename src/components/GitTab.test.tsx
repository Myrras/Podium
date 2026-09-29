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
import { GitTab } from "./GitTab";

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
});

describe("GitTab", () => {
  it("fetches on mount and opens the board on click", () => {
    render(<GitTab projectId={P} />);
    expect(issuesBoard).toHaveBeenCalledWith(P, true);
    fireEvent.click(screen.getByText("Git"));
    expect(useLayoutStore.getState().openBoard).toEqual({ projectId: P });
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
    const { rerender } = render(<GitTab projectId={P} />);
    expect(screen.getByText("2 open · 1 review")).toBeTruthy();
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
    rerender(<GitTab projectId={P} />);
    expect(
      screen.getByLabelText("No GitHub or GitLab remote found."),
    ).toBeTruthy();
  });
});
