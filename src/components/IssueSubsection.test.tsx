import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../ipc/commands", () => ({
  issuesList: vi.fn(() => new Promise(() => {})),
  toIpcError: (e: unknown) => e,
}));
vi.mock("../lib/links", () => ({ openExternalLink: vi.fn() }));

import { issuesList } from "../ipc/commands";
import type { IssueInfo } from "../ipc/types";
import { openExternalLink } from "../lib/links";
import { useIssueStore, type ProjectIssues } from "../state/issueStore";
import { useLayoutStore } from "../state/layoutStore";
import { IssueSubsection } from "./IssueSubsection";

const P = "proj-1";

function issue(number: number): IssueInfo {
  return {
    number,
    title: `Issue ${number}`,
    url: `https://github.com/acme/widgets/issues/${number}`,
    body: "",
    labels: ["bug"],
    updatedAt: "2026-09-01T10:00:00Z",
  };
}

function seed(p: Partial<ProjectIssues>) {
  useIssueStore.setState(
    {
      byProject: {
        [P]: { issues: [], error: null, loading: false, fetchedAt: 1, ...p },
      },
    },
    false,
  );
}

beforeEach(() => {
  vi.mocked(issuesList).mockClear();
  vi.mocked(openExternalLink).mockClear();
  useIssueStore.setState({ byProject: {} }, false);
  useLayoutStore.setState({ openIssue: null });
});

describe("IssueSubsection", () => {
  it("renders rows; a click opens the detail pane, the action opens the browser", () => {
    seed({ issues: [issue(7), issue(9)] });
    render(<IssueSubsection projectId={P} />);
    fireEvent.click(screen.getByText("#7 Issue 7"));
    expect(useLayoutStore.getState().openIssue).toEqual({
      projectId: P,
      number: 7,
    });
    expect(openExternalLink).not.toHaveBeenCalled();
    fireEvent.click(screen.getByLabelText("Open #7 in browser"));
    expect(openExternalLink).toHaveBeenCalledWith(
      "https://github.com/acme/widgets/issues/7",
    );
    expect(screen.getByText("#9 Issue 9")).toBeTruthy();
  });

  it("fetches on mount and shows loading on first load", () => {
    render(<IssueSubsection projectId={P} />);
    expect(issuesList).toHaveBeenCalledWith(P);
    expect(screen.getByText("Loading…")).toBeTruthy();
  });

  it("shows the empty state", () => {
    seed({});
    render(<IssueSubsection projectId={P} />);
    expect(screen.getByText("No open issues assigned to you.")).toBeTruthy();
  });

  it.each([
    ["issuesNoRemote", "No GitHub or GitLab remote found."],
    ["issuesGhUnavailable", "gh auth login"],
    ["issuesUnsupportedHost", "only supported for GitHub"],
  ])("shows the %s state without rows or retry", (kind, text) => {
    seed({ issues: [issue(1)], error: { kind, message: "x" } });
    render(<IssueSubsection projectId={P} />);
    expect(screen.getByRole("status").textContent).toContain(text);
    expect(screen.queryByText("#1 Issue 1")).toBeNull();
    expect(screen.queryByText("Retry")).toBeNull();
  });

  it("keeps stale rows under a fetch error and offers retry", () => {
    seed({
      issues: [issue(1)],
      error: { kind: "issuesFetch", message: "Could not fetch issues: boom" },
    });
    render(<IssueSubsection projectId={P} />);
    expect(screen.getByText("Could not fetch issues: boom")).toBeTruthy();
    expect(screen.getByText("#1 Issue 1")).toBeTruthy();
    expect(screen.getByText("Retry")).toBeTruthy();
  });

  it("keeps rows visible while refreshing and the button reports busy", () => {
    seed({ issues: [issue(1)], loading: true });
    render(<IssueSubsection projectId={P} />);
    expect(screen.getByText("#1 Issue 1")).toBeTruthy();
    expect(
      screen.getByLabelText("Refresh issues").getAttribute("aria-busy"),
    ).toBe("true");
  });
});
