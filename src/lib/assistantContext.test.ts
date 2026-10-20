/**
 * The assistant's selection context: what Cmd/Ctrl-selected items and a text
 * selection turn into, that merely *opening* an item is not context, and that
 * an oversized text selection is dropped.
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
const TODO_B = "44444444-4444-4444-8444-444444444444";
const AGENT = "33333333-3333-4333-8333-333333333333";

function todo(id: string, text: string) {
  return {
    id,
    projectId: PROJECT,
    text,
    description: null,
    done: false,
    createdAt: "2026-01-01T00:00:00Z",
    doneAt: null,
    archived: false,
    archivedAt: null,
    links: [],
    comments: [],
    assignedAgent: null,
  };
}

describe("useAssistantContext", () => {
  beforeEach(() => {
    useLayoutStore.setState({ openTodo: null, openScratchpad: null });
    useProcessStore.setState({ processes: [], activeProcessId: null });
    useTodoStore.setState({ todosByProject: {} });
    useScratchpadStore.setState({ scratchpadsByProject: {} });
    useSelectionStore.setState({ text: "", items: [] });
  });

  it("is empty with nothing selected", () => {
    const { result } = renderHook(() => useAssistantContext());
    expect(result.current).toEqual([]);
  });

  it("ignores an item that is merely open in the work area", () => {
    useLayoutStore.setState({ openTodo: { projectId: PROJECT, todoId: TODO } });
    useProcessStore.setState({ activeProcessId: AGENT, processes: [] });
    useTodoStore.setState({
      todosByProject: { [PROJECT]: [todo(TODO, "Fix the login form")] },
    });

    const { result } = renderHook(() => useAssistantContext());
    expect(result.current).toEqual([]);
  });

  it("carries every selected to-do, in pick order", () => {
    useTodoStore.setState({
      todosByProject: {
        [PROJECT]: [
          todo(TODO, "Fix the login form"),
          todo(TODO_B, "Ship the release notes"),
        ],
      },
    });
    useSelectionStore.setState({
      items: [
        { kind: "todo", projectId: PROJECT, id: TODO_B },
        { kind: "todo", projectId: PROJECT, id: TODO },
      ],
    });

    const { result } = renderHook(() => useAssistantContext());
    expect(result.current.map((i) => i.label)).toEqual([
      "To-do: Ship the release notes",
      "To-do: Fix the login form",
    ]);
    const [item] = result.current;
    expect(item.text).toContain(TODO_B);
    expect(contextAttachment(item)).toEqual({
      kind: "text",
      name: item.label,
      text: item.text,
    });
  });

  it("drops a selected to-do that no longer exists", () => {
    useSelectionStore.setState({
      items: [{ kind: "todo", projectId: PROJECT, id: TODO }],
    });
    const { result } = renderHook(() => useAssistantContext());
    expect(result.current).toEqual([]);
  });

  it("names a selected agent", () => {
    useProcessStore.setState({
      activeProcessId: null,
      processes: [
        {
          id: AGENT,
          projectId: PROJECT,
          name: "auth-agent",
          kind: { kind: "agent", adapter: "claude-code" },
          status: { state: "notStarted" },
          restartPolicy: "never",
          command: "claude",
          worktree: null,
          color: null,
          viewing: false,
        },
      ],
    });
    useSelectionStore.setState({
      items: [{ kind: "process", projectId: PROJECT, id: AGENT }],
    });

    const { result } = renderHook(() => useAssistantContext());
    expect(result.current[0].label).toBe("Agent: auth-agent");
    expect(result.current[0].text).toContain(AGENT);
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
