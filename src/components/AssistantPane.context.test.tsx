/**
 * Selection context in the composer: the open to-do shows as a chip, rides
 * along as a text attachment, and can be dismissed before sending.
 *
 * Unlike the main pane test this mocks Tauri's `invoke` rather than the
 * command layer, so the to-do/process stores keep their real imports.
 */

import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(() => Promise.resolve(null)),
}));

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args?: unknown) => invoke(cmd, args),
  Channel: class {},
}));

import type { Attachment } from "../ipc/types";
import { useAssistantStore } from "../state/assistantStore";
import { useLayoutStore } from "../state/layoutStore";
import { useProcessStore } from "../state/processStore";
import { useTodoStore } from "../state/todoStore";
import { AssistantPane } from "./AssistantPane";

const PROJECT = "11111111-1111-4111-8111-111111111111";
const TODO = "22222222-2222-4222-8222-222222222222";

/** The `attachments` argument of the last `assistant_send` call. */
function sentAttachments(): Attachment[] {
  const calls = invoke.mock.calls.filter(
    (c: unknown[]) => c[0] === "assistant_send",
  );
  const last = calls[calls.length - 1] as [
    string,
    { attachments: Attachment[] },
  ];
  return last[1].attachments;
}

describe("AssistantPane selection context", () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockImplementation((cmd: string) => {
      switch (cmd) {
        case "assistant_send":
          return Promise.resolve(true);
        case "assistant_autonomy_get":
          return Promise.resolve("confirm-destructive");
        case "assistant_transcript":
          return Promise.resolve([]);
        default:
          return Promise.resolve(null);
      }
    });
    useAssistantStore.setState({ messages: [], pending: [], hydrated: false });
    useProcessStore.setState({ processes: [], activeProcessId: null });
    useLayoutStore.setState({
      openTodo: { projectId: PROJECT, todoId: TODO },
      openScratchpad: null,
    });
    useTodoStore.setState({
      todosByProject: {
        [PROJECT]: [
          {
            id: TODO,
            projectId: PROJECT,
            text: "Fix the login form",
            description: null,
            done: false,
            createdAt: "2026-01-01T00:00:00Z",
            doneAt: null,
            archived: false,
            archivedAt: null,
            links: [],
            comments: [],
            assignedAgent: null,
          },
        ],
      },
    });
  });

  it("shows the open to-do as a chip and sends it as context", async () => {
    const user = userEvent.setup();
    render(<AssistantPane />);

    expect(screen.getByText("To-do: Fix the login form")).toBeInTheDocument();

    await user.type(
      screen.getByLabelText("Message the assistant"),
      "what is left here?",
    );
    await user.click(screen.getByLabelText("Send"));

    await waitFor(() => expect(sentAttachments()).toHaveLength(1));
    const [context] = sentAttachments();
    expect(context).toMatchObject({ kind: "text" });
    expect((context as { text: string }).text).toContain(TODO);
  });

  it("drops the chip when the user dismisses it", async () => {
    const user = userEvent.setup();
    render(<AssistantPane />);

    await user.click(
      screen.getByLabelText("Remove To-do: Fix the login form from context"),
    );
    expect(
      screen.queryByText("To-do: Fix the login form"),
    ).not.toBeInTheDocument();

    await user.type(screen.getByLabelText("Message the assistant"), "hello");
    await user.click(screen.getByLabelText("Send"));

    await waitFor(() =>
      expect(invoke.mock.calls.some(([cmd]) => cmd === "assistant_send")).toBe(
        true,
      ),
    );
    expect(sentAttachments()).toEqual([]);
  });
});
