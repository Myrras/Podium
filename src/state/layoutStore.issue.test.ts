import { beforeEach, describe, expect, it } from "vitest";

import { useLayoutStore } from "./layoutStore";
import { useProcessStore } from "./processStore";

beforeEach(() => {
  useLayoutStore.setState({
    openTodo: null,
    openScratchpad: null,
    openIssue: null,
  });
});

describe("openIssue", () => {
  it("is mutually exclusive with to-do, scratchpad and the focused process", () => {
    const s = useLayoutStore.getState();
    s.openTodoInWorkArea("p", "t");
    s.openIssueInWorkArea("p", 7);
    expect(useLayoutStore.getState().openTodo).toBeNull();
    expect(useLayoutStore.getState().openIssue).toEqual({
      projectId: "p",
      number: 7,
    });

    useLayoutStore.getState().openScratchpadInWorkArea("p", "s");
    expect(useLayoutStore.getState().openIssue).toBeNull();

    useLayoutStore.getState().openIssueInWorkArea("p", 8);
    expect(useLayoutStore.getState().openScratchpad).toBeNull();

    useProcessStore.getState().setActiveProcess("proc");
    expect(useLayoutStore.getState().openIssue).toBeNull();
  });
});
