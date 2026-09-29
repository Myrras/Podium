import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../ipc/commands", () => ({ toIpcError: (e: unknown) => e }));
vi.mock("../lib/links", () => ({ openExternalLink: vi.fn() }));

import type { BoardItem } from "../ipc/types";
import { openExternalLink } from "../lib/links";
import { useTodoStore } from "../state/todoStore";
import { IssueDetailPane } from "./IssueDetailPane";

const P = "proj-1";
const issue: BoardItem = {
  kind: "issue",
  number: 7,
  title: "Fix the widget",
  url: "https://github.com/acme/widgets/issues/7",
  body: "Some **bold** text",
  labels: ["bug", "ui"],
  updatedAt: "2026-09-01T10:00:00Z",
  column: "open",
  draft: false,
  author: "sam",
};

beforeEach(() => {
  vi.mocked(openExternalLink).mockClear();
  useTodoStore.setState({ todosByProject: {} });
});

describe("IssueDetailPane", () => {
  it("renders title, reference, labels, author and markdown body", () => {
    render(
      <IssueDetailPane
        projectId={P}
        item={issue}
        itemRef="#7"
        onClose={() => {}}
      />,
    );
    expect(screen.getByText("#7 Fix the widget")).toBeTruthy();
    expect(screen.getByText("bug")).toBeTruthy();
    expect(screen.getByText("ui")).toBeTruthy();
    expect(screen.getByText("by sam")).toBeTruthy();
    expect(screen.getByText("bold").tagName).toBe("STRONG");
    fireEvent.click(screen.getByLabelText("Open #7 in browser"));
    expect(openExternalLink).toHaveBeenCalledWith(issue.url);
  });

  it("says so when the item dropped off the board", () => {
    render(
      <IssueDetailPane
        projectId={P}
        item={undefined}
        itemRef="#7"
        onClose={() => {}}
      />,
    );
    expect(screen.getByText(/no longer on the board/)).toBeTruthy();
    expect(screen.queryByLabelText("Open #7 in browser")).toBeNull();
  });

  it("closes", () => {
    const onClose = vi.fn();
    render(
      <IssueDetailPane
        projectId={P}
        item={issue}
        itemRef="#7"
        onClose={onClose}
      />,
    );
    fireEvent.click(screen.getByLabelText("Close details"));
    expect(onClose).toHaveBeenCalled();
  });
});
