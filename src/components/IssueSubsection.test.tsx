import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../ipc/commands", () => ({
  issuesList: vi.fn(() => new Promise(() => {})),
  issuesSettingsGet: vi.fn(() => new Promise(() => {})),
  issuesProjectSetupSet: vi.fn(),
  toIpcError: (e: unknown) => e,
}));
vi.mock("../lib/links", () => ({ openExternalLink: vi.fn() }));

import { issuesList, issuesProjectSetupSet } from "../ipc/commands";
import type { IssueInfo } from "../ipc/types";
import { openExternalLink } from "../lib/links";
import { useSettingsRequestStore } from "../state/settingsRequestStore";
import { useIssueStore, type ProjectIssues } from "../state/issueStore";
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
  useIssueStore.setState({ byProject: {}, overview: null }, false);
});

describe("IssueSubsection", () => {
  it("renders rows and opens the browser on click", () => {
    seed({ issues: [issue(7), issue(9)] });
    render(<IssueSubsection projectId={P} root="/work/widgets" />);
    fireEvent.click(screen.getByText("#7 Issue 7"));
    expect(openExternalLink).toHaveBeenCalledWith(
      "https://github.com/acme/widgets/issues/7",
    );
    expect(screen.getByText("#9 Issue 9")).toBeTruthy();
  });

  it("fetches on mount and shows loading on first load", () => {
    render(<IssueSubsection projectId={P} root="/work/widgets" />);
    expect(issuesList).toHaveBeenCalledWith(P);
    expect(screen.getByText("Loading…")).toBeTruthy();
  });

  it("shows the empty state", () => {
    seed({});
    render(<IssueSubsection projectId={P} root="/work/widgets" />);
    expect(screen.getByText("No open issues assigned to you.")).toBeTruthy();
  });

  it.each([
    ["issuesNoRemote", "No GitHub or GitLab remote found."],
    ["issuesGhUnavailable", "gh auth login"],
    ["issuesUnsupportedHost", "No setup in Settings matches"],
  ])("shows the %s state without rows or retry", (kind, text) => {
    seed({ issues: [issue(1)], error: { kind, message: "x" } });
    render(<IssueSubsection projectId={P} root="/work/widgets" />);
    expect(screen.getByRole("status").textContent).toContain(text);
    expect(screen.queryByText("#1 Issue 1")).toBeNull();
    expect(screen.queryByText("Retry")).toBeNull();
  });

  it.each([
    ["issuesGitlabNotConfigured", "This GitLab setup has no token."],
    ["issuesGitlabAuth", "GitLab rejected the token."],
    ["issuesUnsupportedHost", "No setup in Settings matches"],
    ["issuesHostMismatch", "not for github.com"],
  ])("%s links to Settings → Issues", (kind, text) => {
    seed({ error: { kind, message: "Setup is not for github.com" } });
    useSettingsRequestStore.setState({ request: null });
    render(<IssueSubsection projectId={P} root="/work/widgets" />);
    expect(screen.getByRole("status").textContent).toContain(text);
    expect(screen.queryByText("Retry")).toBeNull();
    fireEvent.click(screen.getByText("Check Settings"));
    expect(useSettingsRequestStore.getState().request).toBe("issues");
  });

  it("keeps stale rows under a fetch error and offers retry", () => {
    seed({
      issues: [issue(1)],
      error: { kind: "issuesFetch", message: "Could not fetch issues: boom" },
    });
    render(<IssueSubsection projectId={P} root="/work/widgets" />);
    expect(screen.getByText("Could not fetch issues: boom")).toBeTruthy();
    expect(screen.getByText("#1 Issue 1")).toBeTruthy();
    expect(screen.getByText("Retry")).toBeTruthy();
  });

  it("links to Settings → Issues from the header", () => {
    seed({});
    useSettingsRequestStore.setState({ request: null });
    render(<IssueSubsection projectId={P} root="/work/widgets" />);
    fireEvent.click(screen.getByLabelText("Issue settings"));
    expect(useSettingsRequestStore.getState().request).toBe("issues");
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
    vi.mocked(issuesList).mockResolvedValue([]);
    render(<IssueSubsection projectId={P} root="/work/widgets" />);
    const picker = screen.getByLabelText("Issue setup");
    expect(picker).toHaveProperty("value", "");
    await waitFor(() =>
      expect(useIssueStore.getState().byProject[P]?.loading).toBe(false),
    );
    vi.mocked(issuesList).mockClear();
    fireEvent.change(picker, { target: { value: "s1" } });
    expect(issuesProjectSetupSet).toHaveBeenCalledWith(P, "s1");
    await waitFor(() => expect(picker).toHaveProperty("value", "s1"));
    expect(issuesList).toHaveBeenCalledWith(P);
    vi.mocked(issuesList).mockImplementation(() => new Promise(() => {}));
  });

  it("keeps rows visible while refreshing and the button reports busy", () => {
    seed({ issues: [issue(1)], loading: true });
    render(<IssueSubsection projectId={P} root="/work/widgets" />);
    expect(screen.getByText("#1 Issue 1")).toBeTruthy();
    expect(
      screen.getByLabelText("Refresh issues").getAttribute("aria-busy"),
    ).toBe("true");
  });
});
