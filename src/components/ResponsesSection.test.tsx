import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

// The store imports the IPC layer; nothing here should reach it, since the
// row actions are replaced with spies below.
vi.mock("../ipc/commands", () => ({}));
// Rooms need full process fixtures; the rows only read a bot variant from them.
vi.mock("../lib/floor/rooms", () => ({
  useRooms: () => [],
  roomOf: () => null,
}));

import type { ProcessInfo, ResponseItem } from "../ipc/types";
import { useAssistantStore } from "../state/assistantStore";
import { useLayoutStore } from "../state/layoutStore";
import { useProcessStore } from "../state/processStore";
import { formatRelativeTime } from "../lib/dateFormat";
import { ResponsesSection } from "./ResponsesSection";

const NOW = Date.now();
// Filed after the fixture items: what a superseding item looks like.
const LATER = new Date(NOW).toISOString();

function item(overrides: Partial<ResponseItem> = {}): ResponseItem {
  return {
    id: "r-1",
    kind: "needsYou",
    text: "Keep the old redirect, or drop it?",
    project: "Webshop",
    agent: "Checkout fix",
    agentColor: "#3e63dd",
    todo: "Fix the checkout loop",
    source: null,
    processId: "proc-1",
    todoId: "todo-1",
    createdAt: new Date(NOW - 2 * 60_000).toISOString(),
    key: "process:proc-1",
    fingerprint: "",
    state: "open",
    reason: "other",
    sender: null,
    label: "Checkout fix · Webshop · Fix the checkout loop",
    ...overrides,
  };
}

const dismissResponse = vi.fn(() => Promise.resolve());
const parkResponse = vi.fn(() => Promise.resolve());
const unparkResponse = vi.fn(() => Promise.resolve());
const replyResponse = vi.fn(() => Promise.resolve(true));

describe("ResponsesSection", () => {
  beforeEach(() => {
    dismissResponse.mockClear();
    parkResponse.mockClear();
    unparkResponse.mockClear();
    replyResponse.mockClear();
    replyResponse.mockResolvedValue(true);
    useLayoutStore.setState({ mode: "project" });
    useProcessStore.setState({
      processes: [{ id: "proc-1" } as ProcessInfo],
    });
    useAssistantStore.setState({
      responses: [
        item(),
        item({
          id: "r-2",
          kind: "fyi",
          text: "Finished, PR open for review.",
          agent: null,
          agentColor: null,
          project: null,
          todo: null,
          source: "Slack",
          processId: null,
          todoId: null,
          createdAt: new Date(NOW - 3 * 3_600_000).toISOString(),
        }),
      ],
      dismissResponse,
      parkResponse,
      unparkResponse,
      replyResponse,
    });
  });

  it("renders each item with its name, place and age", () => {
    render(<ResponsesSection />);

    expect(screen.getByRole("button", { name: /Needs you/ })).toHaveTextContent(
      "2",
    );
    expect(screen.getByText("Checkout fix")).toBeInTheDocument();
    expect(screen.getByText("Webshop")).toBeInTheDocument();
    expect(screen.getByText("2m")).toBeInTheDocument();
    // No agent or project: the source stands in for them.
    expect(screen.getByText("Slack")).toBeInTheDocument();
    expect(screen.getByText("3h")).toBeInTheDocument();
    // No kind tag and no Show on floor button any more.
    expect(screen.queryByText("FYI")).toBeNull();
    expect(screen.queryByRole("button", { name: "Show on floor" })).toBeNull();
  });

  it("leaves blank name parts out of the row's name, as the label does", () => {
    useAssistantStore.setState({
      responses: [item({ agent: "  ", project: " ", todo: "Fix the loop" })],
    });
    render(<ResponsesSection />);
    expect(screen.getByText("Fix the loop").textContent).toBe("Fix the loop");
  });

  it("always renders, with an empty message when nothing is open", () => {
    useAssistantStore.setState({ responses: [] });
    render(<ResponsesSection />);
    expect(screen.getByText("Nothing needs you.")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Parked/ })).toBeNull();
  });

  it("groups open and parked items, parked collapsed by default", async () => {
    const user = userEvent.setup();
    useAssistantStore.setState({
      responses: [
        item(),
        item({ id: "r-9", key: "k-9", state: "parked", text: "Later." }),
      ],
    });
    render(<ResponsesSection />);

    expect(screen.getByRole("button", { name: /Needs you/ })).toHaveTextContent(
      "1",
    );
    const parked = screen.getByRole("button", { name: /Parked/ });
    expect(parked).toHaveTextContent("1");
    expect(parked).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText(/^parked /)).not.toBeInTheDocument();

    await user.click(parked);
    // A parked row is one line: name and "parked <age>", no text.
    expect(screen.getByText(/^parked /)).toBeInTheDocument();
  });

  it("still shows the empty message next to a parked group", () => {
    useAssistantStore.setState({ responses: [item({ state: "parked" })] });
    render(<ResponsesSection />);
    expect(screen.getByText("Nothing needs you.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Parked/ })).toBeInTheDocument();
  });

  it("parks, brings back and dismisses by key", async () => {
    const user = userEvent.setup();
    useAssistantStore.setState({
      responses: [
        item(),
        item({ id: "r-9", key: "k-9", state: "parked", text: "Later." }),
      ],
    });
    render(<ResponsesSection />);

    await user.click(
      screen.getByRole("button", { name: "Park until it changes" }),
    );
    expect(parkResponse).toHaveBeenCalledWith("process:proc-1");
    await user.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(dismissResponse).toHaveBeenCalledWith("process:proc-1");

    await user.click(screen.getByRole("button", { name: /Parked/ }));
    await user.click(screen.getByRole("button", { name: "Bring back" }));
    expect(unparkResponse).toHaveBeenCalledWith("k-9");
    const dismiss = screen.getAllByRole("button", { name: "Dismiss" });
    await user.click(dismiss[1]);
    expect(dismissResponse).toHaveBeenLastCalledWith("k-9");
  });

  it("collapses and expands the list", async () => {
    const user = userEvent.setup();
    render(<ResponsesSection />);
    const header = screen.getByRole("button", { name: /Needs you/ });

    await user.click(header);
    expect(header).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText("Checkout fix")).not.toBeInTheDocument();

    await user.click(header);
    expect(screen.getByText("Checkout fix")).toBeInTheDocument();
  });

  it("sends a reply to the item it answers", async () => {
    const user = userEvent.setup();
    render(<ResponsesSection />);

    await user.click(screen.getAllByRole("button", { name: "Reply" })[0]);
    const send = screen.getByLabelText("Send reply");
    // Nothing to send yet.
    expect(send).toBeDisabled();
    await user.type(screen.getByLabelText("Reply text"), "   ");
    expect(send).toBeDisabled();

    await user.type(screen.getByLabelText("Reply text"), "Drop it");
    await user.click(send);
    expect(replyResponse).toHaveBeenCalledWith("r-1", "   Drop it");
    expect(screen.queryByLabelText("Reply text")).not.toBeInTheDocument();
  });

  it("sends on Enter and cancels on Escape", async () => {
    const user = userEvent.setup();
    render(<ResponsesSection />);
    const replyBtn = screen.getAllByRole("button", { name: "Reply" })[0];

    await user.click(replyBtn);
    await user.type(screen.getByLabelText("Reply text"), "never mind");
    await user.keyboard("{Escape}");
    expect(screen.queryByLabelText("Reply text")).not.toBeInTheDocument();
    expect(replyResponse).not.toHaveBeenCalled();

    await user.click(replyBtn);
    await user.type(screen.getByLabelText("Reply text"), "XLSX{Enter}");
    expect(replyResponse).toHaveBeenCalledWith("r-1", "XLSX");
  });

  it("sends a reply once, however often it is submitted while in flight", async () => {
    let finish: (sent: boolean) => void = () => {};
    replyResponse.mockImplementationOnce(
      () => new Promise<boolean>((resolve) => (finish = resolve)),
    );
    const user = userEvent.setup();
    render(<ResponsesSection />);

    await user.click(screen.getAllByRole("button", { name: "Reply" })[0]);
    await user.type(screen.getByLabelText("Reply text"), "XLSX{Enter}");
    expect(screen.getByLabelText("Send reply")).toBeDisabled();
    await user.keyboard("{Enter}");
    expect(replyResponse).toHaveBeenCalledOnce();

    await act(async () => finish(true));
    expect(screen.queryByLabelText("Reply text")).not.toBeInTheDocument();
  });

  it("carries a half-typed reply over to the item that replaces its row", async () => {
    const user = userEvent.setup();
    render(<ResponsesSection />);

    await user.click(screen.getAllByRole("button", { name: "Reply" })[0]);
    await user.type(screen.getByLabelText("Reply text"), "Keep it for");
    // A newer item about the same agent supersedes r-1.
    act(() =>
      useAssistantStore.setState({
        responses: [
          item({ id: "r-3", text: "Redirect still needed?", createdAt: LATER }),
        ],
      }),
    );

    expect(screen.getByText("Redirect still needed?")).toBeInTheDocument();
    expect(screen.getByLabelText("Reply text")).toHaveValue("Keep it for");
  });

  it("drops a cleared draft under every key, so it never resurfaces", async () => {
    const user = userEvent.setup();
    render(<ResponsesSection />);

    await user.click(screen.getAllByRole("button", { name: "Reply" })[0]);
    await user.type(screen.getByLabelText("Reply text"), "half a thought");
    // Superseded via the agent only; the draft follows, then is cancelled.
    act(() =>
      useAssistantStore.setState({
        responses: [item({ id: "r-3", todoId: null, createdAt: LATER })],
      }),
    );
    expect(screen.getByLabelText("Reply text")).toHaveValue("half a thought");
    await user.type(screen.getByLabelText("Reply text"), "{Escape}");

    // A later item about the original to-do must not bring it back.
    act(() =>
      useAssistantStore.setState({
        responses: [item({ id: "r-4", processId: null, createdAt: LATER })],
      }),
    );
    expect(screen.queryByLabelText("Reply text")).not.toBeInTheDocument();
  });

  it("focuses the reply box only when Reply is clicked", async () => {
    const user = userEvent.setup();
    render(<ResponsesSection />);

    await user.click(screen.getAllByRole("button", { name: "Reply" })[0]);
    expect(screen.getByLabelText("Reply text")).toHaveFocus();
    await user.type(screen.getByLabelText("Reply text"), "Keep it");

    // The user moves on to the main composer; a superseding item that
    // carries the draft over must not pull focus back.
    const composer = document.createElement("textarea");
    document.body.append(composer);
    composer.focus();
    act(() =>
      useAssistantStore.setState({
        responses: [
          item({ id: "r-3", text: "Redirect still needed?", createdAt: LATER }),
        ],
      }),
    );
    expect(screen.getByLabelText("Reply text")).toHaveValue("Keep it");
    expect(composer).toHaveFocus();
    composer.remove();
  });

  it("keeps a draft on its own row when an FYI about the same agent sits beside it", async () => {
    const user = userEvent.setup();
    useAssistantStore.setState({ responses: [item()] });
    render(<ResponsesSection />);

    await user.click(screen.getByRole("button", { name: "Reply" }));
    await user.type(screen.getByLabelText("Reply text"), "Drop it");
    // An FYI never replaces the open question: both rows stay.
    act(() =>
      useAssistantStore.setState({
        responses: [
          item({
            id: "r-5",
            kind: "fyi",
            text: "Tests green.",
            createdAt: LATER,
          }),
          item(),
        ],
      }),
    );
    expect(screen.getAllByLabelText("Reply text")).toHaveLength(1);
    expect(screen.getByLabelText("Reply text")).toHaveValue("Drop it");

    // The FYI's own reply box starts empty and answers the FYI.
    const [fyiReply, questionReply] = screen.getAllByRole("button", {
      name: "Reply",
    });
    expect(questionReply).toHaveAttribute("aria-pressed", "true");
    await user.click(fyiReply);
    const [fyiBox, questionBox] = screen.getAllByLabelText("Reply text");
    expect(fyiBox).toHaveValue("");
    expect(questionBox).toHaveValue("Drop it");
    await user.type(fyiBox, "Thanks{Enter}");
    expect(replyResponse).toHaveBeenCalledWith("r-5", "Thanks");
    await user.type(questionBox, "{Enter}");
    expect(replyResponse).toHaveBeenCalledWith("r-1", "Drop it");
  });

  it("keeps the ages current", () => {
    vi.useFakeTimers({ now: NOW });
    try {
      useAssistantStore.setState({
        responses: [item({ createdAt: new Date(NOW).toISOString() })],
      });
      render(<ResponsesSection />);
      expect(screen.getByText("now")).toBeInTheDocument();

      act(() => vi.advanceTimersByTime(5 * 60_000));
      expect(screen.getByText("5m")).toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("formatRelativeTime compact", () => {
  it("compacts an age to one unit", () => {
    const ago = (ms: number) => new Date(NOW - ms).toISOString();
    const age = (ms: number) =>
      formatRelativeTime(ago(ms), new Date(NOW), { compact: true });
    expect(age(10_000)).toBe("now");
    expect(age(5 * 60_000)).toBe("5m");
    expect(age(3 * 3_600_000)).toBe("3h");
    expect(age(2 * 86_400_000)).toBe("2d");
  });
});
