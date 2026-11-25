import { act, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => undefined)),
}));
vi.mock("../../lib/floor/camera", () => ({ showAgent: vi.fn() }));
// The avatars only read a bot variant from the rooms.
vi.mock("../../lib/floor/rooms", () => ({
  useRooms: () => [],
  roomOf: () => null,
}));

import type { ResponseItem } from "../../ipc/types";
import { showAgent } from "../../lib/floor/camera";
import { useAssistantStore } from "../../state/assistantStore";
import { useFloorStore } from "../../state/floorStore";
import { FrontDesk } from "./FrontDesk";

const initialAssistant = useAssistantStore.getState();

function item(n: number, overrides: Partial<ResponseItem> = {}): ResponseItem {
  return {
    id: `r-${n}`,
    kind: "needsYou",
    text: `Question ${n} about the retry fix`,
    project: "Harbor",
    agent: `Agent ${n}`,
    agentColor: "#3e63dd",
    todo: null,
    source: null,
    processId: `proc-${n}`,
    todoId: null,
    // Higher n = newer.
    createdAt: new Date(Date.UTC(2026, 0, 1, 10, n)).toISOString(),
    key: `agent:proc-${n}`,
    fingerprint: "",
    state: "open",
    reason: "question",
    sender: null,
    label: `Agent ${n} · Harbor`,
    ...overrides,
  };
}

const parkResponse = vi.fn(() => Promise.resolve());
const dismissResponse = vi.fn(() => Promise.resolve());
const replyResponse = vi.fn(() => Promise.resolve(true));

function seed(responses: ResponseItem[]) {
  useAssistantStore.setState(
    {
      ...initialAssistant,
      enabled: true,
      responses,
      parkResponse,
      dismissResponse,
      replyResponse,
    },
    true,
  );
}

const figures = () =>
  Array.from(document.querySelectorAll("button[aria-label^='Agent']")).map(
    (b) => b.getAttribute("aria-label"),
  );

beforeEach(() => {
  vi.clearAllMocks();
  useFloorStore.getState().reset();
});

describe("FrontDesk", () => {
  it("queues open items newest first, at most six, then +N", () => {
    seed([1, 2, 3, 4, 5, 6, 7, 8].map((n) => item(n)));
    render(<FrontDesk />);
    expect(figures()).toEqual([
      "Agent 8",
      "Agent 7",
      "Agent 6",
      "Agent 5",
      "Agent 4",
      "Agent 3",
    ]);
    expect(screen.getByText("+2")).toBeInTheDocument();
    expect(screen.getByText("8 need you")).toBeInTheDocument();
  });

  it("leaves parked items out of the queue and the count", () => {
    seed([item(1), item(2, { state: "parked" })]);
    render(<FrontDesk />);
    expect(figures()).toEqual(["Agent 1"]);
    expect(screen.getByText("1 needs you")).toBeInTheDocument();
  });

  it("says nothing needs you when the queue is empty", () => {
    seed([item(1, { state: "parked" })]);
    render(<FrontDesk />);
    expect(figures()).toEqual([]);
    expect(screen.getByText("Nothing needs you")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Nothing needs you" }).nextSibling,
    ).toHaveTextContent("Idle");
  });

  it("rings the agent's desk while a figure is hovered or focused", () => {
    seed([item(1)]);
    render(<FrontDesk />);
    const fig = screen.getByRole("button", { name: "Agent 1" });
    fireEvent.mouseEnter(fig.parentElement!);
    expect(useFloorStore.getState().hoverDesk).toBe("proc-1");
    fireEvent.mouseLeave(fig.parentElement!);
    expect(useFloorStore.getState().hoverDesk).toBeNull();

    fireEvent.focus(fig);
    expect(useFloorStore.getState().hoverDesk).toBe("proc-1");
    fireEvent.blur(fig);
    expect(useFloorStore.getState().hoverDesk).toBeNull();
  });

  it("drops the ring when the hovered figure leaves the queue", () => {
    seed([item(1)]);
    render(<FrontDesk />);
    fireEvent.mouseEnter(
      screen.getByRole("button", { name: "Agent 1" }).parentElement!,
    );
    act(() => seed([]));
    expect(useFloorStore.getState().hoverDesk).toBeNull();
  });

  it("goes to the agent on a figure click", async () => {
    seed([item(1)]);
    render(<FrontDesk />);
    await userEvent.click(screen.getByRole("button", { name: "Agent 1" }));
    expect(showAgent).toHaveBeenCalledWith("proc-1");
  });

  it("parks, dismisses and replies from the card with the item", async () => {
    const user = userEvent.setup();
    seed([item(1)]);
    render(<FrontDesk />);
    const card = within(
      screen.getByRole("button", { name: "Agent 1" }).parentElement!,
    );
    expect(card.getByText("Asks")).toBeInTheDocument();
    expect(
      card.getByText("Question 1 about the retry fix"),
    ).toBeInTheDocument();

    await user.click(
      card.getByRole("button", { name: "Park until it changes" }),
    );
    expect(parkResponse).toHaveBeenCalledWith("agent:proc-1");
    await user.click(card.getByRole("button", { name: "Dismiss" }));
    expect(dismissResponse).toHaveBeenCalledWith("agent:proc-1");

    await user.click(card.getByRole("button", { name: "Reply" }));
    await user.type(card.getByLabelText("Reply text"), "Rebase it{Enter}");
    expect(replyResponse).toHaveBeenCalledWith("r-1", "Rebase it");
  });

  it("has no Reply on a message", () => {
    seed([
      item(1, {
        reason: "message",
        processId: null,
        sender: "Dana",
        source: "Slack",
        key: "msg:slack:c1:t1",
      }),
    ]);
    render(<FrontDesk />);
    const card = within(
      screen.getByRole("button", { name: "Dana" }).parentElement!,
    );
    expect(card.queryByRole("button", { name: "Reply" })).toBeNull();
    expect(card.getByRole("button", { name: "Dismiss" })).toBeInTheDocument();
  });

  it("opens chat from the bot and the list from the count", async () => {
    const onOpenChat = vi.fn();
    const onOpenList = vi.fn();
    seed([item(1)]);
    render(<FrontDesk onOpenChat={onOpenChat} onOpenList={onOpenList} />);
    await userEvent.click(screen.getByRole("button", { name: "Open chat" }));
    expect(onOpenChat).toHaveBeenCalledTimes(1);
    await userEvent.click(screen.getByRole("button", { name: "1 needs you" }));
    expect(onOpenList).toHaveBeenCalledTimes(1);
  });
});
