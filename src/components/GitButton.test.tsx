import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../ipc/commands", () => ({
  issuesBoard: vi.fn(() => new Promise(() => {})),
  issuesBoardCached: vi.fn(() => Promise.resolve(null)),
  issuesSettingsGet: vi.fn(() => new Promise(() => {})),
  toIpcError: (e: unknown) => e,
}));

import { issuesBoard } from "../ipc/commands";
import type { BoardItem } from "../ipc/types";
import { resetAutoFetchPacing, useIssueStore } from "../state/issueStore";
import { useLayoutStore } from "../state/layoutStore";
import { useProjectStore } from "../state/projectStore";
import { GitButton } from "./GitButton";

const P = "proj-1";

const card = (column: string, number: number): BoardItem => ({
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
  movable: false,
  linked: [],
  assignees: [],
  approvals: null,
});

/** One setup, so the board is fetched (no setup = nothing is fetched). */
const CONFIGURED = {
  setups: [
    {
      id: "s1",
      name: "GitHub",
      provider: "github" as const,
      url: "https://github.com",
      tokenSet: false,
    },
  ],
  projects: {},
};

beforeEach(() => {
  vi.mocked(issuesBoard).mockClear();
  resetAutoFetchPacing();
  useIssueStore.setState({ byProject: {}, overview: CONFIGURED }, false);
  useLayoutStore.setState({ openBoard: null });
  useProjectStore.setState({ activeProjectId: null });
});

describe("GitButton", () => {
  it("fetches on mount and opens the board on click", async () => {
    render(<GitButton projectId={P} />);
    await waitFor(() => expect(issuesBoard).toHaveBeenCalledWith(P, true));
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
            columns: [],
            source: null,
            hint: null,
            me: null,
            items: [
              card("open", 1),
              card("label:doing", 2),
              card("review", 3),
              card("done", 4),
            ],
          },
          error: null,
          loading: true,
          fetchedAt: 1,
          mine: true,
        },
      },
    });
    const { rerender } = render(<GitButton projectId={P} />);
    // The pill shows the open-card count at rest.
    expect(
      screen.getByRole("button", { name: "Git board — 3 open" }).textContent,
    ).toBe("3");
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
    expect(btn.textContent).toBe("");
  });

  it("fetches nothing and shows no warning while no setup exists", async () => {
    useIssueStore.setState({ overview: { setups: [], projects: {} } });
    render(<GitButton projectId={P} />);
    await waitFor(() =>
      expect(useIssueStore.getState().byProject[P]?.error?.kind).toBe(
        "issuesNotConfigured",
      ),
    );
    expect(issuesBoard).not.toHaveBeenCalled();
    const button = screen.getByRole("button", { name: "Git board" });
    expect(button).not.toHaveAttribute("data-error");
  });
});
