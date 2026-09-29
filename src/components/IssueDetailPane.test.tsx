import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../ipc/commands", () => ({ toIpcError: (e: unknown) => e }));
vi.mock("../lib/links", () => ({ openExternalLink: vi.fn() }));

import type { IssueInfo } from "../ipc/types";
import { openExternalLink } from "../lib/links";
import { useIssueStore } from "../state/issueStore";
import { useLayoutStore } from "../state/layoutStore";
import { useTodoStore } from "../state/todoStore";
import { IssueDetailPane } from "./IssueDetailPane";

const P = "proj-1";
const issue: IssueInfo = {
  number: 7,
  title: "Fix the widget",
  url: "https://github.com/acme/widgets/issues/7",
  body: "Some **bold** text",
  labels: ["bug", "ui"],
  updatedAt: "2026-09-01T10:00:00Z",
};

function seed(issues: IssueInfo[]) {
  useIssueStore.setState(
    {
      byProject: {
        [P]: { issues, error: null, loading: false, fetchedAt: 1 },
      },
    },
    false,
  );
}

beforeEach(() => {
  vi.mocked(openExternalLink).mockClear();
  useTodoStore.setState({ todosByProject: {} });
  useLayoutStore.setState({ openIssue: { projectId: P, number: 7 } });
});

describe("IssueDetailPane", () => {
  it("renders title, number, labels and markdown body", () => {
    seed([issue]);
    render(<IssueDetailPane projectId={P} number={7} />);
    expect(screen.getByText("#7 Fix the widget")).toBeTruthy();
    expect(screen.getByText("bug")).toBeTruthy();
    expect(screen.getByText("ui")).toBeTruthy();
    expect(screen.getByText("bold").tagName).toBe("STRONG");
    fireEvent.click(screen.getByLabelText("Open #7 in browser"));
    expect(openExternalLink).toHaveBeenCalledWith(issue.url);
  });

  it("says so when the issue dropped out of the list", () => {
    seed([]);
    render(<IssueDetailPane projectId={P} number={7} />);
    expect(screen.getByText(/no longer open or assigned/)).toBeTruthy();
    expect(screen.queryByLabelText("Open #7 in browser")).toBeNull();
  });

  it("closes", () => {
    seed([issue]);
    render(<IssueDetailPane projectId={P} number={7} />);
    fireEvent.click(screen.getByLabelText("Close issue"));
    expect(useLayoutStore.getState().openIssue).toBeNull();
  });
});
