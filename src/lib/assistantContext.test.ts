/**
 * The assistant's selection context: what the work-area selection and a text
 * selection turn into, and that a selection inside the assistant pane itself
 * is ignored.
 */

import { renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";

import { useLayoutStore } from "../state/layoutStore";
import { useProcessStore } from "../state/processStore";
import { useScratchpadStore } from "../state/scratchpadStore";
import { useTodoStore } from "../state/todoStore";
import { contextAttachment, useAssistantContext } from "./assistantContext";
import { setSelectedText, useSelectionStore } from "./selection";

const PROJECT = "11111111-1111-4111-8111-111111111111";
const TODO = "22222222-2222-4222-8222-222222222222";

describe("useAssistantContext", () => {
  beforeEach(() => {
    useLayoutStore.setState({ openTodo: null, openScratchpad: null });
    useProcessStore.setState({ processes: [], activeProcessId: null });
    useTodoStore.setState({ todosByProject: {} });
    useScratchpadStore.setState({ scratchpadsByProject: {} });
    useSelectionStore.setState({ text: "" });
  });

  it("is empty with nothing selected", () => {
    const { result } = renderHook(() => useAssistantContext());
    expect(result.current).toEqual([]);
  });

  it("carries the open to-do's id and text", () => {
    useLayoutStore.setState({ openTodo: { projectId: PROJECT, todoId: TODO } });
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

    const { result } = renderHook(() => useAssistantContext());
    expect(result.current).toHaveLength(1);
    const [item] = result.current;
    expect(item.label).toBe("To-do: Fix the login form");
    expect(item.text).toContain(TODO);
    expect(item.text).toContain("Fix the login form");
    expect(contextAttachment(item)).toEqual({
      kind: "text",
      name: item.label,
      text: item.text,
    });
  });

  it("names the focused agent", () => {
    useProcessStore.setState({
      activeProcessId: "33333333-3333-4333-8333-333333333333",
      processes: [
        {
          id: "33333333-3333-4333-8333-333333333333",
          projectId: PROJECT,
          name: "auth-agent",
          kind: { kind: "agent", adapter: "claude-code" },
          status: { state: "notStarted" },
          restartPolicy: "never",
          command: "claude",
          worktree: null,
          color: null,
        },
      ],
    });

    const { result } = renderHook(() => useAssistantContext());
    expect(result.current[0].label).toBe("Agent: auth-agent");
    expect(result.current[0].text).toContain(
      "33333333-3333-4333-8333-333333333333",
    );
  });

  it("carries selected text", () => {
    setSelectedText("  thread 'main' panicked  ");
    const { result } = renderHook(() => useAssistantContext());
    expect(result.current[0].label).toBe("Selected text");
    expect(result.current[0].text).toContain("thread 'main' panicked");
  });

  it("ignores a selection longer than the inline cap", () => {
    setSelectedText("x".repeat(100_001));
    expect(useSelectionStore.getState().text).toBe("");
  });
});
