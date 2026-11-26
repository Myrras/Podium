import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// The store imports the IPC layer; nothing here should reach it, since the
// row actions are replaced with spies below.
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("../ipc/commands", () => ({}));
vi.mock("../lib/floor/camera", () => ({ showAgent: vi.fn() }));
// Rooms need full process fixtures; the rows only read a bot variant from them.
vi.mock("../lib/floor/rooms", () => ({
  useRooms: () => [],
  roomOf: () => null,
}));

import type { ProcessInfo, ResponseItem } from "../ipc/types";
import { showAgent } from "../lib/floor/camera";
import { useAssistantStore } from "../state/assistantStore";
import { useLayoutStore } from "../state/layoutStore";
import { useProcessStore } from "../state/processStore";
import { OverviewRow } from "./OverviewRow";
import { Popover } from "./Popover";

function item(overrides: Partial<ResponseItem> = {}): ResponseItem {
  return {
    id: "r-1",
    kind: "needsYou",
    text: "Rebase on main or merge?",
    project: "Harbor",
    agent: "Checkout retry fix",
    agentColor: "#3e63dd",
    todo: null,
    source: null,
    processId: "proc-1",
    todoId: null,
    createdAt: new Date(Date.now() - 4 * 60_000).toISOString(),
    key: "agent:proc-1",
    fingerprint: "",
    state: "open",
    reason: "question",
    sender: null,
    label: "Checkout retry fix · Harbor",
    ...overrides,
  };
}

const dismissResponse = vi.fn(() => Promise.resolve());
const parkResponse = vi.fn(() => Promise.resolve());
const unparkResponse = vi.fn(() => Promise.resolve());
const replyResponse = vi.fn(() => Promise.resolve(true));
const setActiveProcess = vi.fn();

function renderRow(it: ResponseItem) {
  return render(
    <ul>
      <OverviewRow item={it} draft={null} setDraft={() => {}} />
    </ul>,
  );
}

describe("OverviewRow", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useLayoutStore.setState({ mode: "project" });
    useProcessStore.setState({
      processes: [{ id: "proc-1" } as ProcessInfo],
      setActiveProcess,
    });
    useAssistantStore.setState({
      dismissResponse,
      parkResponse,
      unparkResponse,
      replyResponse,
    });
  });

  it.each([
    ["question", "Asks", "?"],
    ["crashed", "Crashed", "!"],
    ["review", "Ready to review", "✓"],
  ] as const)("shows the word and mark for %s", (reason, word, mark) => {
    const { container } = renderRow(item({ reason }));
    expect(screen.getByText(word).tagName).toBe("B");
    expect(screen.getByText(word).parentElement).toHaveTextContent(
      `${word} in Harbor`,
    );
    expect(container.querySelector("li")).toHaveAttribute(
      "data-reason",
      reason,
    );
    expect(screen.getByText(mark)).toHaveAttribute("aria-hidden", "true");
  });

  it("keeps other plain: no word, no mark", () => {
    renderRow(item({ reason: "other" }));
    expect(screen.getByText("Harbor")).toBeInTheDocument();
    for (const t of ["Asks", "Crashed", "Ready to review", "?", "!", "✓"])
      expect(screen.queryByText(t)).toBeNull();
  });

  it("shows a message's sender and its place, with only Reply and Done", async () => {
    const user = userEvent.setup();
    renderRow(
      item({
        reason: "message",
        agent: null,
        project: null,
        processId: null,
        sender: "Alex Example",
        source: "Slack, direct message",
        key: "msg:slack:d1:t1",
      }),
    );
    expect(screen.getByText("Alex Example")).toBeInTheDocument();
    expect(screen.getByText("Slack, direct message")).toBeInTheDocument();
    expect(
      screen.getAllByRole("button").map((b) => b.getAttribute("aria-label")),
    ).toEqual(["Reply", "Done: mark as read"]);
    await user.click(
      screen.getByRole("button", { name: "Done: mark as read" }),
    );
    expect(dismissResponse).toHaveBeenCalledWith("msg:slack:d1:t1");
  });

  it("keeps Reply and Done on a parked message", () => {
    renderRow(
      item({
        reason: "message",
        state: "parked",
        processId: null,
        sender: "Alex Example",
        key: "msg:slack:d1:t1",
      }),
    );
    expect(
      screen.getAllByRole("button").map((b) => b.getAttribute("aria-label")),
    ).toEqual(["Reply", "Done: mark as read"]);
  });

  it("shows a parked item on one line with its own actions", async () => {
    const user = userEvent.setup();
    renderRow(item({ state: "parked" }));
    expect(screen.getByText("Checkout retry fix")).toBeInTheDocument();
    expect(screen.getByText("parked 4m")).toBeInTheDocument();
    expect(screen.queryByText("Rebase on main or merge?")).toBeNull();
    expect(screen.queryByText("Asks")).toBeNull();
    expect(screen.queryByRole("button", { name: "Reply" })).toBeNull();

    await user.click(screen.getByRole("button", { name: "Bring back" }));
    expect(unparkResponse).toHaveBeenCalledWith("agent:proc-1");
    await user.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(dismissResponse).toHaveBeenCalledWith("agent:proc-1");
  });

  it("parks and dismisses an open item by key", async () => {
    const user = userEvent.setup();
    renderRow(item());
    await user.click(
      screen.getByRole("button", { name: "Park until it changes" }),
    );
    expect(parkResponse).toHaveBeenCalledWith("agent:proc-1");
    await user.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(dismissResponse).toHaveBeenCalledWith("agent:proc-1");
    // The actions never navigate.
    expect(setActiveProcess).not.toHaveBeenCalled();
  });

  it("opens the agent on click in Project mode and dismisses its item", async () => {
    const user = userEvent.setup();
    renderRow(item());
    await user.click(screen.getByText("Rebase on main or merge?"));
    expect(setActiveProcess).toHaveBeenCalledWith("proc-1");
    expect(showAgent).not.toHaveBeenCalled();
    expect(dismissResponse).toHaveBeenCalledWith("agent:proc-1");
  });

  it("moves the floor to the agent on Enter in Floor mode and dismisses its item", async () => {
    const user = userEvent.setup();
    useLayoutStore.setState({ mode: "floor" });
    const { container } = renderRow(item());
    container.querySelector("li")!.focus();
    await user.keyboard("{Enter}");
    expect(showAgent).toHaveBeenCalledWith("proc-1");
    expect(setActiveProcess).not.toHaveBeenCalled();
    expect(dismissResponse).toHaveBeenCalledWith("agent:proc-1");
  });

  it("keeps the assistant's own item about an agent when opening it", async () => {
    const user = userEvent.setup();
    renderRow(item({ key: "process:proc-1", reason: "other" }));
    await user.click(screen.getByText("Rebase on main or merge?"));
    expect(setActiveProcess).toHaveBeenCalledWith("proc-1");
    expect(dismissResponse).not.toHaveBeenCalled();
  });

  it("goes nowhere once the agent's process is gone, and has no Show on floor", async () => {
    const user = userEvent.setup();
    useLayoutStore.setState({ mode: "floor" });
    useProcessStore.setState({ processes: [] });
    const { container } = renderRow(item());
    expect(container.querySelector("li")).not.toHaveAttribute("tabindex");
    expect(screen.queryByRole("button", { name: "Show on floor" })).toBeNull();
    await user.click(screen.getByText("Rebase on main or merge?"));
    expect(showAgent).not.toHaveBeenCalled();
  });

  it("Esc in the reply field closes the field, not the enclosing popover", async () => {
    const user = userEvent.setup();
    const anchor = document.createElement("button");
    document.body.append(anchor);
    const onClose = vi.fn();
    function Row() {
      const [draft, setDraft] = useState<string | null>("");
      return (
        <ul>
          <OverviewRow item={item()} draft={draft} setDraft={setDraft} />
        </ul>
      );
    }
    render(
      <Popover anchor={anchor} title="Items" onClose={onClose}>
        <Row />
      </Popover>,
    );
    await user.type(screen.getByLabelText("Reply text"), "{Escape}");
    expect(screen.queryByLabelText("Reply text")).toBeNull();
    expect(onClose).not.toHaveBeenCalled();
  });
});
