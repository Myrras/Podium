import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../ipc/commands", () => ({
  issuesBoardMove: vi.fn(() => Promise.resolve()),
  toIpcError: (e: unknown) => e,
}));
vi.mock("../lib/links", () => ({ openExternalLink: vi.fn() }));

import { issuesBoardMove } from "../ipc/commands";
import type { BoardItem } from "../ipc/types";
import { useIssueStore } from "../state/issueStore";
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
  movable: false,
  linked: [],
  assignees: [],
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

  describe("Move to", () => {
    const withBoard = (item: BoardItem) =>
      useIssueStore.setState(
        {
          byProject: {
            [P]: {
              board: {
                provider: "github",
                items: [item],
                columns: [
                  {
                    id: "open",
                    title: "Open",
                    color: null,
                    accepts: ["issue"],
                  },
                  {
                    id: "doing",
                    title: "Doing",
                    color: null,
                    accepts: ["issue"],
                  },
                  {
                    id: "review",
                    title: "Review",
                    color: null,
                    accepts: ["pr"],
                  },
                ],
                source: null,
                hint: null,
                me: null,
                prCounts: { open: 0, merged: 0 },
              },
              error: null,
              loading: false,
              fetchedAt: 1,
              mine: true,
            },
          },
        },
        false,
      );
    const pane = (item: BoardItem) =>
      render(
        <IssueDetailPane
          projectId={P}
          item={item}
          itemRef="#7"
          onClose={() => {}}
        />,
      );

    it("lists valid columns and moves through the store", () => {
      const it = { ...issue, movable: true };
      withBoard(it);
      pane(it);
      const select = screen.getByLabelText("Move to") as HTMLSelectElement;
      expect(Array.from(select.options).map((o) => o.textContent)).toEqual([
        "Choose column…",
        "Doing",
      ]);
      fireEvent.change(select, { target: { value: "doing" } });
      expect(issuesBoardMove).toHaveBeenCalledWith(P, "issue", 7, "doing");
    });

    it("is hidden when the item is not movable", () => {
      withBoard(issue);
      pane(issue);
      expect(screen.queryByLabelText("Move to")).toBeNull();
    });
  });

  it("lists linked PRs and opens them in the browser", () => {
    const it = {
      ...issue,
      linked: [
        {
          number: 20,
          title: "Fix it",
          url: "https://github.com/acme/widgets/pull/20",
          draft: false,
          merged: true,
        },
      ],
    };
    render(
      <IssueDetailPane
        projectId={P}
        item={it}
        itemRef="#7"
        onClose={() => {}}
      />,
    );
    expect(screen.getByText("Linked pull requests")).toBeTruthy();
    fireEvent.click(screen.getByLabelText("Open #20 in browser"));
    expect(openExternalLink).toHaveBeenCalledWith(it.linked[0].url);
    expect(screen.getByText(/merged/)).toBeTruthy();
  });
});
