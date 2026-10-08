import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

// The store imports the IPC layer; nothing here should reach it, since the
// row actions are replaced with spies below.
vi.mock("../ipc/commands", () => ({}));

import type { ResponseItem } from "../ipc/types";
import { useAssistantStore } from "../state/assistantStore";
import { shortAge } from "../lib/dateFormat";
import { ResponsesSection } from "./ResponsesSection";

const NOW = Date.now();

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
    label: "Checkout fix · Webshop · Fix the checkout loop",
    ...overrides,
  };
}

const dismissResponse = vi.fn(() => Promise.resolve());
const replyResponse = vi.fn(() => Promise.resolve(true));

describe("ResponsesSection", () => {
  beforeEach(() => {
    dismissResponse.mockClear();
    replyResponse.mockClear();
    replyResponse.mockResolvedValue(true);
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
      replyResponse,
    });
  });

  it("renders each item with its context, kind and age", () => {
    render(<ResponsesSection />);

    expect(screen.getByRole("button", { name: /Responses/ })).toHaveTextContent(
      "2",
    );
    expect(screen.getByText("Checkout fix")).toBeInTheDocument();
    expect(
      screen.getByText(/Webshop · Fix the checkout loop/),
    ).toBeInTheDocument();
    expect(screen.getByText("Needs you")).toBeInTheDocument();
    expect(screen.getByText("2m")).toBeInTheDocument();
    // No agent or project: the source stands in for them.
    expect(screen.getByText("Slack")).toBeInTheDocument();
    expect(screen.getByText("FYI")).toBeInTheDocument();
    expect(screen.getByText("3h")).toBeInTheDocument();
  });

  it("renders nothing when there are no items", () => {
    useAssistantStore.setState({ responses: [] });
    const { container } = render(<ResponsesSection />);
    expect(container).toBeEmptyDOMElement();
  });

  it("collapses and expands the list", async () => {
    const user = userEvent.setup();
    render(<ResponsesSection />);
    const header = screen.getByRole("button", { name: /Responses/ });

    await user.click(header);
    expect(header).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText("Checkout fix")).not.toBeInTheDocument();

    await user.click(header);
    expect(screen.getByText("Checkout fix")).toBeInTheDocument();
  });

  it("ticks an item off", async () => {
    const user = userEvent.setup();
    render(<ResponsesSection />);

    await user.click(screen.getAllByLabelText("Mark as read")[0]);
    expect(dismissResponse).toHaveBeenCalledWith("r-1");
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
        responses: [item({ id: "r-3", text: "Redirect still needed?" })],
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
        responses: [item({ id: "r-3", todoId: null })],
      }),
    );
    expect(screen.getByLabelText("Reply text")).toHaveValue("half a thought");
    await user.type(screen.getByLabelText("Reply text"), "{Escape}");

    // A later item about the original to-do must not bring it back.
    act(() =>
      useAssistantStore.setState({
        responses: [item({ id: "r-4", processId: null })],
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
        responses: [item({ id: "r-3", text: "Redirect still needed?" })],
      }),
    );
    expect(screen.getByLabelText("Reply text")).toHaveValue("Keep it");
    expect(composer).toHaveFocus();
    composer.remove();
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

describe("shortAge", () => {
  it("compacts an age to one unit", () => {
    const ago = (ms: number) => new Date(NOW - ms).toISOString();
    expect(shortAge(ago(10_000), NOW)).toBe("now");
    expect(shortAge(ago(5 * 60_000), NOW)).toBe("5m");
    expect(shortAge(ago(3 * 3_600_000), NOW)).toBe("3h");
    expect(shortAge(ago(2 * 86_400_000), NOW)).toBe("2d");
  });
});
