import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../ipc/commands", () => ({
  todoAdd: vi.fn(),
  todoUpdate: vi.fn(),
  todoAddLink: vi.fn(),
  issuesBoardUnassign: vi.fn(),
  adaptersList: vi.fn(async () => []),
  agentSettingsGet: vi.fn(async () => ({
    adapters: [],
    defaultAdapter: null,
  })),
  toIpcError: (e: unknown) => ({ message: String(e) }),
}));
vi.mock("../lib/links", () => ({ openExternalLink: vi.fn() }));

import {
  issuesBoardUnassign,
  todoAdd,
  todoAddLink,
  todoUpdate,
} from "../ipc/commands";
import type { Board, BoardItem, TodoInfo } from "../ipc/types";
import { useIssueStore } from "../state/issueStore";
import { useTodoStore } from "../state/todoStore";
import { IssueActions } from "./IssueActions";

const P = "proj-1";
const issue: BoardItem = {
  kind: "issue",
  number: 7,
  title: "Fix the widget",
  url: "https://github.com/acme/widgets/issues/7",
  body: "Body text",
  labels: [],
  updatedAt: "2026-09-01T10:00:00Z",
  column: "open",
  draft: false,
  author: "",
  movable: false,
  linked: [],
  assignees: [],
  approvals: null,
};

function todo(over: Partial<TodoInfo> = {}): TodoInfo {
  return {
    id: "t1",
    projectId: P,
    text: "#7 Fix the widget",
    description: null,
    done: false,
    createdAt: "2026-09-01T10:00:00Z",
    doneAt: null,
    archived: false,
    archivedAt: null,
    links: [],
    comments: [],
    assignedAgent: null,
    sessions: [],
    ...over,
  };
}

beforeEach(() => {
  vi.mocked(todoAdd).mockReset();
  vi.mocked(todoUpdate).mockReset();
  vi.mocked(todoAddLink).mockReset();
  useTodoStore.setState({ todosByProject: {} });
});

describe("IssueActions", () => {
  it("turns the issue into a to-do: add, description, pinned link", async () => {
    vi.mocked(todoAdd).mockResolvedValue(todo());
    vi.mocked(todoUpdate).mockResolvedValue(todo({ description: "Body text" }));
    vi.mocked(todoAddLink).mockResolvedValue(todo());
    render(
      <IssueActions projectId={P} issue={issue} itemRef="#7" className="a" />,
    );
    fireEvent.click(screen.getByLabelText("Create a to-do for #7"));
    await waitFor(() => expect(todoAddLink).toHaveBeenCalled());
    expect(todoAdd).toHaveBeenCalledWith(P, "#7 Fix the widget");
    expect(todoUpdate).toHaveBeenCalledWith(P, "t1", {
      description: "Body text",
    });
    expect(todoAddLink).toHaveBeenCalledWith(P, "t1", issue.url, "#7");
  });

  it("hides the to-do action when an active to-do already links the issue", () => {
    useTodoStore.setState({
      todosByProject: {
        [P]: [
          todo({ links: [{ id: "l1", url: issue.url, label: "#7" }] as never }),
        ],
      },
    });
    render(
      <IssueActions projectId={P} issue={issue} itemRef="#7" className="a" />,
    );
    expect(screen.queryByLabelText("Create a to-do for #7")).toBeNull();
    expect(screen.getByLabelText("To-do exists for #7")).toBeTruthy();
  });

  it("still offers the to-do when the linking to-do is archived", () => {
    useTodoStore.setState({
      todosByProject: {
        [P]: [
          todo({
            archived: true,
            links: [{ id: "l1", url: issue.url, label: "#7" }] as never,
          }),
        ],
      },
    });
    render(
      <IssueActions projectId={P} issue={issue} itemRef="#7" className="a" />,
    );
    expect(screen.getByLabelText("Create a to-do for #7")).toBeTruthy();
  });

  it("opens the agent modal prefilled with the issue", async () => {
    render(
      <IssueActions projectId={P} issue={issue} itemRef="#7" className="a" />,
    );
    fireEvent.click(screen.getByLabelText("Start an agent on #7"));
    expect(
      ((await screen.findByLabelText("Name")) as HTMLInputElement).value,
    ).toBe("#7 Fix the widget");
    const prompt = screen.getByLabelText(
      "Initial prompt (optional)",
    ) as HTMLTextAreaElement;
    // Link only: the agent fetches the body itself over MCP.
    expect(prompt.value).toBe(`Work on issue ${issue.url}`);
  });

  it("prefills a merge request with its own reference and wording", async () => {
    const mr: BoardItem = {
      ...issue,
      kind: "pr",
      number: 41,
      url: "https://git.example.test/grp/proj/-/merge_requests/41",
    };
    render(
      <IssueActions projectId={P} issue={mr} itemRef="!41" className="a" />,
    );
    fireEvent.click(screen.getByLabelText("Start an agent on !41"));
    expect(
      ((await screen.findByLabelText("Name")) as HTMLInputElement).value,
    ).toBe("!41 Fix the widget");
    expect(
      (
        screen.getByLabelText(
          "Initial prompt (optional)",
        ) as HTMLTextAreaElement
      ).value,
    ).toMatch(/^Work on pull request https:\/\/git\.example\.test\S*$/);
  });

  it("unassigns me only when I am an assignee", async () => {
    const seedMe = (me: string | null) =>
      useIssueStore.setState({
        byProject: { [P]: { board: { me } as Board } as never },
      });
    vi.mocked(issuesBoardUnassign).mockResolvedValue(undefined);
    const mine = { ...issue, assignees: ["sam", "kim"] };

    seedMe(null);
    const { rerender } = render(
      <IssueActions projectId={P} issue={mine} itemRef="#7" className="a" />,
    );
    expect(screen.queryByLabelText("Unassign me from #7")).toBeNull();

    seedMe("robin");
    rerender(
      <IssueActions projectId={P} issue={mine} itemRef="#7" className="a" />,
    );
    expect(screen.queryByLabelText("Unassign me from #7")).toBeNull();

    seedMe("sam");
    rerender(
      <IssueActions projectId={P} issue={mine} itemRef="#7" className="a" />,
    );
    fireEvent.click(screen.getByLabelText("Unassign me from #7"));
    await waitFor(() =>
      expect(issuesBoardUnassign).toHaveBeenCalledWith(P, "issue", 7),
    );
  });
});
