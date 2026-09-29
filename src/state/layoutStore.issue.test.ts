import { beforeEach, describe, expect, it } from "vitest";

import { useLayoutStore } from "./layoutStore";
import { useProcessStore } from "./processStore";

beforeEach(() => {
  useLayoutStore.setState({
    openTodo: null,
    openScratchpad: null,
    openBoard: null,
  });
});

describe("openBoard", () => {
  it("is mutually exclusive with to-do, scratchpad and the focused process", () => {
    const s = useLayoutStore.getState();
    s.openTodoInWorkArea("p", "t");
    s.openBoardInWorkArea("p");
    expect(useLayoutStore.getState().openTodo).toBeNull();
    expect(useLayoutStore.getState().openBoard).toEqual({ projectId: "p" });

    useLayoutStore.getState().openScratchpadInWorkArea("p", "s");
    expect(useLayoutStore.getState().openBoard).toBeNull();

    useLayoutStore.getState().openBoardInWorkArea("p");
    expect(useLayoutStore.getState().openScratchpad).toBeNull();

    useProcessStore.getState().setActiveProcess("proc");
    expect(useLayoutStore.getState().openBoard).toBeNull();
  });
});
