import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../ipc/commands", () => ({
  issuesBoard: vi.fn(() => new Promise(() => {})),
  issuesBoardCached: vi.fn(() => Promise.resolve(null)),
  issuesBoardMove: vi.fn(() => Promise.resolve()),
  issuesSettingsGet: vi.fn(() => new Promise(() => {})),
  issuesProjectSetupSet: vi.fn(),
  toIpcError: (e: unknown) => e,
}));
vi.mock("../lib/links", () => ({ openExternalLink: vi.fn() }));

import {
  issuesBoard,
  issuesBoardMove,
  issuesProjectSetupSet,
} from "../ipc/commands";
import type { Board, BoardItem, ProjectInfo } from "../ipc/types";
import { openExternalLink } from "../lib/links";
import { useIssueStore, type ProjectBoard } from "../state/issueStore";
import { useLayoutStore } from "../state/layoutStore";
import { useProjectStore } from "../state/projectStore";
import { useSettingsRequestStore } from "../state/settingsRequestStore";
import { GitBoard } from "./GitBoard";

const P = "proj-1";

function item(over: Partial<BoardItem>): BoardItem {
  return {
    kind: "issue",
    number: 1,
    title: "Issue 1",
    url: "https://github.com/acme/widgets/issues/1",
    body: "",
    labels: [],
    updatedAt: "2026-09-01T10:00:00Z",
    column: "open",
    draft: false,
    author: "sam",
    movable: false,
    linked: [],
    assignees: [],
    approvals: null,
    ...over,
  };
}

const githubBoard: Board = {
  provider: "github",
  prCounts: { open: 5, merged: 1200 },
  columns: [
    { id: "open", title: "Open", color: null, accepts: [] },
    { id: "inProgress", title: "In progress", color: null, accepts: [] },
    { id: "review", title: "In review", color: null, accepts: [] },
    { id: "done", title: "Done (14d)", color: null, accepts: [] },
  ],
  source: null,
  hint: null,
  me: null,
  items: [
    item({ number: 7, title: "Fix widget", labels: ["bug"] }),
    item({ number: 8, title: "Being done", column: "inProgress" }),
    item({ kind: "pr", number: 20, title: "Fix 8", column: "review" }),
    item({ kind: "pr", number: 19, title: "Shipped", column: "done" }),
  ],
};

function seed(p: Partial<ProjectBoard>) {
  useIssueStore.setState(
    {
      byProject: {
        [P]: {
          board: githubBoard,
          error: null,
          loading: false,
          fetchedAt: 1,
          mine: true,
          ...p,
        },
      },
    },
    false,
  );
}

const column = (name: string) => within(screen.getByRole("region", { name }));

beforeEach(() => {
  vi.mocked(issuesBoard).mockClear();
  vi.mocked(openExternalLink).mockClear();
  useIssueStore.setState({ byProject: {}, overview: null }, false);
  useLayoutStore.setState({ openBoard: { projectId: P } });
  useProjectStore.setState({
    projects: [{ id: P, root: "/work/widgets" } as ProjectInfo],
  });
  useSettingsRequestStore.setState({ request: null });
});

describe("GitBoard", () => {
  it("sorts cards into their columns with counts", () => {
    seed({});
    render(<GitBoard projectId={P} />);
    expect(column("Open").getByText("Fix widget")).toBeTruthy();
    expect(column("Open").getByText("bug")).toBeTruthy();
    expect(column("In progress").getByText("Being done")).toBeTruthy();
    expect(column("In review").getByText("Fix 8")).toBeTruthy();
    expect(column("Done (14d)").getByText("Shipped")).toBeTruthy();
    expect(screen.getByLabelText("PRs counts").textContent).toBe(
      "PRs: 5 open · 1,200 merged",
    );
    // Mine hides the author; All shows it.
    expect(screen.queryByText("sam")).toBeNull();
  });

  it("renders the project's own columns, source and hint", () => {
    seed({
      board: {
        ...githubBoard,
        columns: [
          { id: "open", title: "Open", color: null, accepts: [] },
          { id: "label:doing", title: "doing", color: "#428BCA", accepts: [] },
          { id: "status:o2", title: "Rework", color: "red", accepts: [] },
        ],
        source: "Development",
        hint: "To use your GitHub Project's columns, run: gh auth refresh -s read:project",
        items: [
          item({ number: 5, title: "On the board", column: "label:doing" }),
        ],
      },
    });
    render(<GitBoard projectId={P} />);
    const doing = screen.getByRole("region", { name: "doing" });
    expect(doing.getAttribute("data-column")).toBe("label:doing");
    expect(doing.style.getPropertyValue("--column-color")).toBe("#428BCA");
    expect(within(doing).getByText("On the board")).toBeTruthy();
    const rework = screen.getByRole("region", { name: "Rework" });
    expect(rework.style.getPropertyValue("--column-color")).toBe(
      "var(--board-red)",
    );
    expect(
      screen
        .getByRole("region", { name: "Open" })
        .style.getPropertyValue("--column-color"),
    ).toBe("");
    expect(screen.queryByRole("region", { name: "In review" })).toBeNull();
    expect(screen.getByText("Development")).toBeTruthy();
    expect(screen.getByRole("note").textContent).toContain(
      "gh auth refresh -s read:project",
    );
  });

  it("says MRs and !n for GitLab", () => {
    seed({
      board: {
        provider: "gitlab",
        prCounts: { open: 2, merged: null },
        columns: githubBoard.columns,
        source: null,
        hint: null,
        me: null,
        items: [
          item({ kind: "pr", number: 41, column: "review", draft: true }),
        ],
      },
    });
    render(<GitBoard projectId={P} />);
    expect(screen.getByLabelText("MRs counts").textContent).toBe(
      "MRs: 2 open · ? merged",
    );
    expect(column("In review").getByText("!41")).toBeTruthy();
    expect(column("In review").getByText("draft")).toBeTruthy();
  });

  it("a card opens the detail drawer; its action opens the browser", () => {
    seed({});
    render(<GitBoard projectId={P} />);
    fireEvent.click(screen.getByText("Fix widget"));
    const drawer = within(screen.getByLabelText("#7 details"));
    expect(drawer.getByText("#7 Fix widget")).toBeTruthy();
    fireEvent.click(drawer.getByLabelText("Open #7 in browser"));
    expect(openExternalLink).toHaveBeenCalledWith(
      "https://github.com/acme/widgets/issues/1",
    );
    fireEvent.click(drawer.getByLabelText("Close details"));
    expect(screen.queryByLabelText("#7 details")).toBeNull();
  });

  it("the scope toggle refetches everything in the repo", async () => {
    seed({});
    render(<GitBoard projectId={P} />);
    expect(screen.getByText("Mine").getAttribute("aria-pressed")).toBe("true");
    fireEvent.click(screen.getByText("All"));
    await waitFor(() => expect(issuesBoard).toHaveBeenCalledWith(P, false));
    expect(useIssueStore.getState().byProject[P].mine).toBe(false);
  });

  it("shows authors in All", () => {
    seed({ mine: false });
    render(<GitBoard projectId={P} />);
    expect(column("Open").getAllByText("sam").length).toBeGreaterThan(0);
  });

  it("shows loading on first load and the empty state after", () => {
    seed({ board: null, loading: true, fetchedAt: 0 });
    const { rerender } = render(<GitBoard projectId={P} />);
    expect(screen.getAllByText("Loading…").length).toBe(4);
    seed({ board: { ...githubBoard, items: [] } });
    rerender(<GitBoard projectId={P} />);
    expect(screen.getByRole("status").textContent).toBe(
      "No open issues or PRs of yours.",
    );
  });

  it.each([
    ["issuesNoRemote", "No GitHub or GitLab remote found."],
    ["issuesGhUnavailable", "gh auth login"],
    ["issuesUnsupportedHost", "No setup in Settings matches"],
  ])("shows the %s state without cards or retry", (kind, text) => {
    seed({ error: { kind, message: "x" } });
    render(<GitBoard projectId={P} />);
    expect(screen.getByRole("status").textContent).toContain(text);
    expect(screen.queryByText("Fix widget")).toBeNull();
    expect(screen.queryByText("Retry")).toBeNull();
  });

  it.each([
    ["issuesGitlabNotConfigured", "This GitLab setup has no token."],
    ["issuesGitlabAuth", "GitLab rejected the token."],
    ["issuesHostMismatch", "not for github.com"],
  ])("%s links to Settings → Issues", (kind, text) => {
    seed({ error: { kind, message: "Setup is not for github.com" } });
    render(<GitBoard projectId={P} />);
    expect(screen.getByRole("status").textContent).toContain(text);
    fireEvent.click(screen.getByText("Check Settings"));
    expect(useSettingsRequestStore.getState().request).toBe("issues");
  });

  it("keeps stale cards under a fetch error and offers retry", () => {
    seed({
      error: { kind: "issuesFetch", message: "GitHub rate limit reached" },
    });
    render(<GitBoard projectId={P} />);
    expect(screen.getByText("GitHub rate limit reached")).toBeTruthy();
    expect(screen.getByText("Fix widget")).toBeTruthy();
    fireEvent.click(screen.getByText("Retry"));
    expect(issuesBoard).toHaveBeenCalledWith(P, true);
  });

  it("keeps cards visible while refreshing and the button reports busy", () => {
    seed({ loading: true });
    render(<GitBoard projectId={P} />);
    expect(screen.getByText("Fix widget")).toBeTruthy();
    expect(
      screen.getByLabelText("Refresh board").getAttribute("aria-busy"),
    ).toBe("true");
  });

  it("gear opens Settings, close clears the board", () => {
    seed({});
    render(<GitBoard projectId={P} />);
    fireEvent.click(screen.getByLabelText("Issue settings"));
    expect(useSettingsRequestStore.getState().request).toBe("issues");
    fireEvent.click(screen.getByLabelText("Close board"));
    expect(useLayoutStore.getState().openBoard).toBeNull();
  });

  it("picks a setup for the project and refetches", async () => {
    seed({});
    const overview = {
      setups: [
        {
          id: "s1",
          name: "Work GitLab",
          provider: "gitlab" as const,
          url: "http://git.example.test",
          tokenSet: true,
        },
      ],
      projects: {},
    };
    useIssueStore.setState({ overview });
    vi.mocked(issuesProjectSetupSet).mockResolvedValue({
      ...overview,
      projects: { "/work/widgets": "s1" },
    });
    render(<GitBoard projectId={P} />);
    const picker = screen.getByLabelText("Issue setup");
    expect(picker).toHaveProperty("value", "");
    fireEvent.change(picker, { target: { value: "s1" } });
    expect(issuesProjectSetupSet).toHaveBeenCalledWith(P, "s1");
    await waitFor(() => expect(picker).toHaveProperty("value", "s1"));
    expect(issuesBoard).toHaveBeenCalledWith(P, true);
  });

  describe("drag & drop", () => {
    const acceptAll = githubBoard.columns.map((c) => ({
      ...c,
      accepts:
        c.id === "review" ? (["pr"] as const) : (["issue", "pr"] as const),
    }));
    const dnd = () =>
      seed({
        board: {
          ...githubBoard,
          columns: acceptAll.map((c) => ({ ...c, accepts: [...c.accepts] })),
          items: [
            item({ number: 7, title: "Fix widget", movable: true }),
            item({ number: 9, title: "Pinned", movable: false }),
          ],
        },
      });
    const card = (t: string) => screen.getByText(t).closest("[draggable]")!;
    const dt = { setData: vi.fn(), effectAllowed: "" };

    it("only movable cards are draggable", () => {
      dnd();
      render(<GitBoard projectId={P} />);
      expect(card("Fix widget").getAttribute("draggable")).toBe("true");
      expect(card("Pinned").getAttribute("draggable")).toBe("false");
    });

    it("marks accepting columns and moves on drop there only", () => {
      dnd();
      vi.mocked(issuesBoardMove).mockClear();
      render(<GitBoard projectId={P} />);
      fireEvent.dragStart(card("Fix widget"), { dataTransfer: dt });
      const region = (n: string) => screen.getByRole("region", { name: n });
      expect(region("In progress").getAttribute("data-drop")).toBe("true");
      // Not the current column, nor one that refuses issues.
      expect(region("Open").getAttribute("data-drop")).toBe("false");
      expect(region("In review").getAttribute("data-drop")).toBe("false");

      expect(
        fireEvent.dragOver(region("In review"), { dataTransfer: dt }),
      ).toBe(true); // not prevented: no drop allowed
      fireEvent.drop(region("In review"), { dataTransfer: dt });
      expect(issuesBoardMove).not.toHaveBeenCalled();

      fireEvent.dragStart(card("Fix widget"), { dataTransfer: dt });
      expect(
        fireEvent.dragOver(region("In progress"), { dataTransfer: dt }),
      ).toBe(false); // prevented: drop allowed
      fireEvent.drop(region("In progress"), { dataTransfer: dt });
      expect(issuesBoardMove).toHaveBeenCalledWith(P, "issue", 7, "inProgress");
      expect(region("In progress").getAttribute("data-drop")).toBeNull();
    });
  });

  it("folds linked PRs into the card and opens them in the browser", () => {
    seed({
      board: {
        ...githubBoard,
        items: [
          item({
            number: 7,
            title: "Fix widget",
            linked: [
              {
                number: 20,
                title: "Fix it",
                url: "https://github.com/acme/widgets/pull/20",
                draft: true,
                merged: false,
                approvals: { by: ["kim", "sam"], required: 2 },
              },
            ],
          }),
        ],
        me: "sam",
      },
    });
    render(<GitBoard projectId={P} />);
    expect(
      column("Open").getByLabelText("Approvals 2/2, including yours").title,
    ).toBe("Approved by kim, you");
    expect(column("Open").getByText("1")).toBeTruthy(); // one card
    expect(column("Open").getByText("draft")).toBeTruthy();
    fireEvent.click(column("Open").getByLabelText("Open #20 in browser"));
    expect(openExternalLink).toHaveBeenCalledWith(
      "https://github.com/acme/widgets/pull/20",
    );
    expect(screen.queryByLabelText("#7 details")).toBeNull();
  });

  it("filters every column by the search, focused with Cmd+F", () => {
    seed({
      board: {
        ...githubBoard,
        items: [
          item({ number: 1, title: "Fix widget", labels: ["bug"] }),
          item({ number: 2, title: "Other thing", column: "done" }),
        ],
      },
    });
    render(<GitBoard projectId={P} />);
    const search = screen.getByLabelText("Search board");
    fireEvent.keyDown(window, { key: "f", metaKey: true });
    expect(document.activeElement).toBe(search);
    fireEvent.change(search, { target: { value: "BUG" } });
    expect(column("Open").getByText("Fix widget")).toBeTruthy();
    expect(column("Done (14d)").queryByText("Other thing")).toBeNull();
    fireEvent.change(search, { target: { value: "#2" } });
    expect(column("Open").queryByText("Fix widget")).toBeNull();
    expect(column("Done (14d)").getByText("Other thing")).toBeTruthy();
    fireEvent.keyDown(search, { key: "Escape" });
    expect(column("Open").getByText("Fix widget")).toBeTruthy();
  });
});
