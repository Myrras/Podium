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
  issuesSettingsGet: vi.fn(() => new Promise(() => {})),
  issuesProjectSetupSet: vi.fn(),
  toIpcError: (e: unknown) => e,
}));
vi.mock("../lib/links", () => ({ openExternalLink: vi.fn() }));

import { issuesBoard, issuesProjectSetupSet } from "../ipc/commands";
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
    ...over,
  };
}

const githubBoard: Board = {
  provider: "github",
  prCounts: { open: 5, merged: 1200 },
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

  it("says MRs and !n for GitLab", () => {
    seed({
      board: {
        provider: "gitlab",
        prCounts: { open: 2, merged: null },
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

  it("the scope toggle refetches everything in the repo", () => {
    seed({});
    render(<GitBoard projectId={P} />);
    expect(screen.getByText("Mine").getAttribute("aria-pressed")).toBe("true");
    fireEvent.click(screen.getByText("All"));
    expect(issuesBoard).toHaveBeenCalledWith(P, false);
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
});
