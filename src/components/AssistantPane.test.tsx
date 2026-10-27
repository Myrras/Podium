import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import css from "./AssistantPane.module.css?raw";

// The file picker is a Tauri plugin: mocked so the picker button resolves
// to fixture paths instead of opening a real dialog.
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(() => Promise.resolve(null)),
}));

// Mock the IPC command layer so the store talks to a fixture, not Tauri.
vi.mock("../ipc/commands", () => ({
  assistantSend: vi.fn(() => Promise.resolve(true)),
  assistantEnabledSet: vi.fn((v: boolean) => Promise.resolve(v)),
  assistantDayStart: vi.fn(() => Promise.resolve(true)),
  assistantAutonomyGet: vi.fn(() => Promise.resolve("confirm-destructive")),
  assistantAutonomySet: vi.fn((mode: string) => Promise.resolve(mode)),
  assistantResolveProposal: vi.fn(() => Promise.resolve()),
  assistantTranscript: vi.fn(() => Promise.resolve([])),
  assistantNewConversation: vi.fn(() => Promise.resolve()),
  assistantMemoryGet: vi.fn(() => Promise.resolve("")),
  assistantMemorySet: vi.fn(() => Promise.resolve()),
  assistantQueueEdit: vi.fn(() => Promise.resolve()),
  assistantQueueRemove: vi.fn(() => Promise.resolve()),
  assistantQueueSendNow: vi.fn(() => Promise.resolve()),
  assistantInterrupt: vi.fn(() => Promise.resolve()),
  assistantResponsesList: vi.fn(() => Promise.resolve([])),
  toIpcError: (e: unknown) => {
    if (typeof e === "object" && e !== null && "kind" in e && "message" in e) {
      return e as { kind: string; message: string };
    }
    return { kind: "unknown", message: String(e) };
  },
}));

import { open as openFileDialog } from "@tauri-apps/plugin-dialog";

import {
  assistantAutonomyGet,
  assistantAutonomySet,
  assistantDayStart,
  assistantEnabledSet,
  assistantInterrupt,
  assistantMemoryGet,
  assistantMemorySet,
  assistantNewConversation,
  assistantQueueEdit,
  assistantQueueRemove,
  assistantQueueSendNow,
  assistantResolveProposal,
  assistantResponsesList,
  assistantSend,
  assistantTranscript,
} from "../ipc/commands";
import { useAssistantStore } from "../state/assistantStore";
import { useLayoutStore } from "../state/layoutStore";
import { AssistantPane } from "./AssistantPane";
import styles from "./AssistantPane.module.css";

describe("AssistantPane", () => {
  beforeEach(() => {
    vi.mocked(assistantSend).mockReset();
    vi.mocked(assistantSend).mockResolvedValue(true);
    vi.mocked(assistantDayStart).mockReset();
    vi.mocked(assistantDayStart).mockResolvedValue(true);
    vi.mocked(assistantAutonomyGet).mockReset();
    vi.mocked(assistantAutonomyGet).mockResolvedValue("confirm-destructive");
    vi.mocked(assistantAutonomySet).mockReset();
    vi.mocked(assistantAutonomySet).mockImplementation((mode) =>
      Promise.resolve(mode),
    );
    vi.mocked(assistantResolveProposal).mockReset();
    vi.mocked(assistantResolveProposal).mockResolvedValue(undefined);
    vi.mocked(assistantTranscript).mockReset();
    vi.mocked(assistantTranscript).mockResolvedValue([]);
    vi.mocked(assistantNewConversation).mockReset();
    vi.mocked(assistantNewConversation).mockResolvedValue(undefined);
    vi.mocked(assistantMemoryGet).mockReset();
    vi.mocked(assistantMemoryGet).mockResolvedValue("");
    vi.mocked(assistantMemorySet).mockReset();
    vi.mocked(assistantMemorySet).mockResolvedValue(undefined);
    vi.mocked(assistantQueueEdit).mockReset();
    vi.mocked(assistantQueueEdit).mockResolvedValue(undefined);
    vi.mocked(assistantQueueRemove).mockReset();
    vi.mocked(assistantQueueRemove).mockResolvedValue(undefined);
    vi.mocked(assistantQueueSendNow).mockReset();
    vi.mocked(assistantQueueSendNow).mockResolvedValue(undefined);
    vi.mocked(assistantInterrupt).mockReset();
    vi.mocked(assistantInterrupt).mockResolvedValue(undefined);
    vi.mocked(assistantResponsesList).mockReset();
    vi.mocked(assistantResponsesList).mockResolvedValue([]);
    vi.mocked(openFileDialog).mockReset();
    vi.mocked(openFileDialog).mockResolvedValue(null);
    useAssistantStore.setState({
      messages: [],
      busy: false,
      activity: null,
      autonomy: "confirm-destructive",
      enabled: true,
      memory: null,
      hydrated: false,
      pending: [],
      responses: [],
    });
  });

  it("sends the typed turn and shows it in the log", async () => {
    const user = userEvent.setup();
    render(<AssistantPane />);

    await user.type(
      screen.getByLabelText("Message the assistant"),
      "what is open?",
    );
    await user.click(screen.getByLabelText("Send"));

    expect(assistantSend).toHaveBeenCalledWith("what is open?", []);
    expect(screen.getByText("what is open?")).toBeInTheDocument();
    // The composer clears so the next turn starts empty.
    expect(screen.getByLabelText("Message the assistant")).toHaveValue("");
    expect(screen.getByRole("status")).toHaveTextContent("thinking…");
  });

  it("grows the composer with the draft and shrinks back after send", async () => {
    // jsdom does no layout, so scrollHeight is always 0 — stub it so the
    // resize effect has something real to react to. This only proves the
    // effect runs and writes the style; the visual max-height cap (CSS
    // `max-height` + `overflow-y: auto` on `.input`) is not covered here.
    let scrollHeight = 32;
    Object.defineProperty(HTMLTextAreaElement.prototype, "scrollHeight", {
      configurable: true,
      get: () => scrollHeight,
    });
    const user = userEvent.setup();
    render(<AssistantPane />);
    const input = screen.getByLabelText(
      "Message the assistant",
    ) as HTMLTextAreaElement;

    scrollHeight = 96;
    await user.type(input, "a long draft that wraps several lines");
    expect(input.style.height).toBe("96px");

    scrollHeight = 32;
    await user.click(screen.getByLabelText("Send"));
    expect(input.style.height).toBe("32px");
  });

  it("sends on Enter but not on Shift+Enter", async () => {
    const user = userEvent.setup();
    render(<AssistantPane />);
    const input = screen.getByLabelText("Message the assistant");

    await user.type(input, "first{Shift>}{Enter}{/Shift}second");
    expect(assistantSend).not.toHaveBeenCalled();

    await user.type(input, "{Enter}");
    expect(assistantSend).toHaveBeenCalledWith("first\nsecond", []);
  });

  it("renders streamed prose, executed actions and failures", () => {
    render(<AssistantPane />);

    act(() => {
      const { apply } = useAssistantStore.getState();
      apply({ type: "message", text: "**Two** agents are running" });
      apply({ type: "tool", name: "mcp__podium__list_processes" });
      apply({ type: "toolFailed", message: "to-do not found" });
      apply({ type: "turnEnd", error: "error_max_turns" });
    });

    expect(screen.getByText("Two")).toBeInTheDocument();
    // The action and its failure fold into one line; the action is named
    // without its MCP plumbing and the error is only a hover.
    expect(screen.getByText("Podium · list processes")).toBeInTheDocument();
    expect(screen.getByText("Ran 1 action · Podium ⚠ 1")).toBeInTheDocument();
    expect(screen.getByTitle("to-do not found")).toHaveTextContent(
      "⚠ action failed: to-do not found",
    );
    expect(
      screen.getByText("The turn failed: error_max_turns"),
    ).toBeInTheDocument();
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("loads the Responses list on mount and shows it above the log", async () => {
    vi.mocked(assistantResponsesList).mockResolvedValue([
      {
        id: "r-1",
        kind: "fyi",
        text: "Finished, PR open for review.",
        project: "Docs site",
        agent: "Search index",
        agentColor: null,
        todo: null,
        source: null,
        processId: "proc-1",
        todoId: null,
        createdAt: new Date().toISOString(),
        label: "Search index · Docs site",
      },
    ]);
    render(<AssistantPane />);

    expect(
      await screen.findByText("Finished, PR open for review."),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("region", { name: "Responses" }),
    ).toBeInTheDocument();
  });

  it("renders a reply naming the item it answers", () => {
    useAssistantStore.setState({
      messages: [
        {
          id: 1,
          role: "reply",
          text: "Invoice export · Webshop\nXLSX, with the VAT column split out",
        },
      ],
    });
    render(<AssistantPane />);

    expect(
      screen.getByText("Re: Invoice export · Webshop"),
    ).toBeInTheDocument();
    // The reply is shown in full (it wraps), so it needs no hover title.
    expect(
      screen.getByText("XLSX, with the VAT column split out"),
    ).toBeInTheDocument();
  });

  it("demotes a proactive nudge instead of showing it as a message to the user", () => {
    render(<AssistantPane />);

    act(() => {
      useAssistantStore.getState().apply({
        type: "notice",
        text: '[Podium] The agent "docs" in project "Elements" finished (exit code 0). Check what it did and tell the user.',
      });
    });

    // A discreet marker, not the raw internal instruction text.
    expect(screen.getByText("Podium nudged the assistant")).toBeInTheDocument();
    expect(screen.queryByText(/Check what it did/)).not.toBeInTheDocument();
  });

  it("shows the running tool in the busy row while it is active", async () => {
    const user = userEvent.setup();
    render(<AssistantPane />);

    await user.type(screen.getByLabelText("Message the assistant"), "hi");
    await user.click(screen.getByLabelText("Send"));
    expect(screen.getByRole("status")).toHaveTextContent("thinking…");

    act(() => {
      useAssistantStore
        .getState()
        .apply({ type: "tool", name: "mcp__podium__list_todos" });
    });
    expect(screen.getByRole("status")).toHaveTextContent(
      "Podium · list todos…",
    );
  });

  it("labels an MCP tool by its server, even when the server name holds underscores", () => {
    render(<AssistantPane />);
    act(() => {
      useAssistantStore.getState().apply({
        type: "tool",
        name: "mcp__claude_ai_Slack__slack_read_channel",
      });
      useAssistantStore.getState().apply({
        type: "tool",
        name: "mcp__claude_ai_Microsoft_365__outlook_email_search",
      });
    });
    expect(screen.getByText("Slack · slack read channel")).toBeInTheDocument();
    expect(
      screen.getByText("Microsoft 365 · outlook email search"),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Ran 2 actions · Slack, Microsoft 365"),
    ).toBeInTheDocument();
  });

  describe("folded activity", () => {
    const apply = (
      e: Parameters<ReturnType<typeof useAssistantStore.getState>["apply"]>[0],
    ) => act(() => useAssistantStore.getState().apply(e));

    it("folds a run into one collapsed line with counts, top servers and failures", () => {
      render(<AssistantPane />);
      for (const name of [
        "mcp__claude_ai_Slack__slack_read_channel",
        "mcp__claude_ai_Slack__slack_read_channel",
        "mcp__podium__list_todos",
        "mcp__claude_ai_Notion__notion_search",
        "mcp__claude_ai_Linear__list_issues",
        "Agent",
      ]) {
        apply({ type: "tool", name });
      }
      apply({ type: "toolFailed", message: "channel_not_found" });

      const summary = screen.getByText(
        "Ran 6 actions · Slack, Podium, Notion, +2 ⚠ 1",
      );
      const details = summary.closest("details") as HTMLDetailsElement;
      expect(details.open).toBe(false);
      expect(document.querySelectorAll("details")).toHaveLength(1);
      // Distinct labels once each, with a count when repeated.
      expect(details).toHaveTextContent("Slack · slack read channel ×2");
      expect(details).toHaveTextContent("Podium · list todos");
      expect(screen.getByTitle("channel_not_found")).toBeInTheDocument();
    });

    it("starts a new run after prose", () => {
      render(<AssistantPane />);
      apply({ type: "tool", name: "Agent" });
      apply({ type: "message", text: "Working on it" });
      apply({ type: "tool", name: "ToolSearch" });
      expect(document.querySelectorAll("details")).toHaveLength(2);
      expect(screen.getByText("Ran 1 action · Agent")).toBeInTheDocument();
      expect(screen.getByText("Ran 1 action · ToolSearch")).toBeInTheDocument();
    });

    it("reads a nudge-only run as Podium nudging, with ×n and the text on hover", () => {
      render(<AssistantPane />);
      apply({ type: "notice", text: "[Podium] first nudge" });
      apply({ type: "notice", text: "[Podium] second nudge" });
      const summary = screen.getByText("Podium nudged the assistant ×2");
      expect(summary.title).toContain("first nudge");
      expect(summary.title).toContain("second nudge");
    });

    it("renders a nudge-only run as a plain line, not a disclosure", () => {
      render(<AssistantPane />);
      apply({ type: "notice", text: "[Podium] first nudge" });
      expect(document.querySelector("details")).toBeNull();
      expect(screen.getByText("Podium nudged the assistant")).toHaveAttribute(
        "data-podium",
        "true",
      );
    });

    it("counts the nudges in an opened mixed run like tools", () => {
      render(<AssistantPane />);
      apply({ type: "notice", text: "[Podium] first nudge" });
      apply({ type: "notice", text: "[Podium] second nudge" });
      apply({ type: "tool", name: "Agent" });
      const item = screen.getByText("Podium nudged the assistant ×2");
      expect(item.tagName).toBe("LI");
      expect(item.title).toContain("first nudge");
      expect(item.title).toContain("second nudge");
    });

    it("marks a run holding a nudge as Podium's own and prefixes its summary", () => {
      render(<AssistantPane />);
      apply({ type: "notice", text: "[Podium] sweep" });
      apply({ type: "tool", name: "mcp__claude_ai_Slack__slack_read_channel" });
      const summary = screen.getByText("Podium check · Ran 1 action · Slack");
      expect(summary.closest("details")).toHaveAttribute("data-podium", "true");
    });

    it("keeps a user turn's actions out of a following nudge's run", () => {
      render(<AssistantPane />);
      // An interrupted turn: actions, no prose, then Podium speaks.
      apply({ type: "tool", name: "Agent" });
      apply({ type: "notice", text: "[Podium] sweep" });
      apply({ type: "tool", name: "mcp__claude_ai_Slack__slack_read_channel" });
      apply({ type: "notice", text: "[Podium] sweep again" });
      const runs = document.querySelectorAll("details");
      // The plain run stays plain; the nudge, its action and the next nudge
      // share one faint run.
      expect(runs).toHaveLength(2);
      expect(runs[0]).not.toHaveAttribute("data-podium");
      expect(screen.getByText("Ran 1 action · Agent")).toBeInTheDocument();
      expect(runs[1]).toHaveAttribute("data-podium", "true");
      expect(runs[1]).toHaveTextContent("Podium check · Ran 1 action · Slack");
    });

    it("leaves a plain run unmarked", () => {
      render(<AssistantPane />);
      apply({ type: "tool", name: "Agent" });
      expect(document.querySelector("details")).not.toHaveAttribute(
        "data-podium",
      );
    });

    it("replays stored failures as part of the run", async () => {
      vi.mocked(assistantTranscript).mockResolvedValue([
        { role: "tool", text: "Agent", at: "2026-01-02T09:00:00Z" },
        { role: "toolFailed", text: "boom", at: "2026-01-02T09:00:01Z" },
      ]);
      render(<AssistantPane />);
      expect(
        await screen.findByText("Ran 1 action · Agent ⚠ 1"),
      ).toBeInTheDocument();
    });

    it("shows a failed turn's reason after 'The turn failed:'", () => {
      render(<AssistantPane />);
      apply({
        type: "turnEnd",
        error: "Usage limit reached: resumes at 15:00",
      });
      expect(
        screen.getByText(
          "The turn failed: Usage limit reached: resumes at 15:00",
        ),
      ).toBeInTheDocument();
    });

    it("keeps 'Start my day' while the transcript holds only activity rows", () => {
      render(<AssistantPane />);
      apply({ type: "notice", text: "[Podium] sweep" });
      apply({ type: "tool", name: "Agent" });
      expect(screen.getByLabelText("Start my day")).toBeInTheDocument();
      apply({ type: "message", text: "hello" });
      expect(screen.queryByLabelText("Start my day")).toBeNull();
    });

    it("splits an action name on the first '__' and collapses '_' runs", () => {
      render(<AssistantPane />);
      apply({
        type: "tool",
        name: "mcp__claude_ai_Notion__pages__search",
      });
      expect(screen.getByText("Ran 1 action · Notion")).toBeInTheDocument();
      expect(
        screen.getByText("Notion · pages search", { exact: false }),
      ).toBeInTheDocument();
    });

    it("caps a long multi-line failure reason", () => {
      render(<AssistantPane />);
      apply({
        type: "turnEnd",
        error: `{\n  "detail": "${"x ".repeat(400)}"\n}`,
      });
      const text = screen.getByText(/^The turn failed: /).textContent ?? "";
      expect(text.length).toBeLessThanOrEqual(330);
      expect(text).not.toContain("\n");
      expect(text.endsWith("…")).toBe(true);
    });

    it("summarises a run of only failures as '⚠ n failed'", () => {
      render(<AssistantPane />);
      apply({ type: "toolFailed", message: "boom" });
      apply({ type: "toolFailed", message: "bang" });
      expect(screen.getByText("⚠ 2 failed")).toBeInTheDocument();
    });

    it("flattens a multi-line failure in its tooltip", () => {
      render(<AssistantPane />);
      apply({ type: "toolFailed", message: "line one\n\nline two" });
      expect(screen.getByTitle("line one line two")).toBeInTheDocument();
    });

    it("clips a huge failure to a one-line preview, full text in the title", () => {
      render(<AssistantPane />);
      const long = `${"word ".repeat(100)}end`;
      apply({ type: "toolFailed", message: long });
      const item = screen.getByTitle(long);
      expect(item.textContent?.endsWith("…")).toBe(true);
      expect(item.textContent!.length).toBeLessThan(120);
    });
  });

  describe("following the tail", () => {
    // jsdom does no layout: stub the geometry the follow logic reads.
    function stubLog(log: HTMLElement, scrollTop: number) {
      let top = scrollTop;
      Object.defineProperty(log, "scrollHeight", {
        configurable: true,
        get: () => 1000,
      });
      Object.defineProperty(log, "clientHeight", {
        configurable: true,
        get: () => 300,
      });
      Object.defineProperty(log, "scrollTop", {
        configurable: true,
        get: () => top,
        set: (v: number) => {
          top = v;
        },
      });
      return () => top;
    }
    const logOf = () =>
      document.querySelector(`.${styles.log}`) as HTMLDivElement;

    it("does not yank a reader who scrolled up, and offers a pill instead", () => {
      render(<AssistantPane />);
      const top = stubLog(logOf(), 100);
      fireEvent.scroll(logOf());
      expect(
        screen.queryByRole("button", { name: "New activity ↓" }),
      ).not.toBeInTheDocument();

      act(() =>
        useAssistantStore.getState().apply({ type: "message", text: "more" }),
      );
      expect(top()).toBe(100);
      expect(
        screen.getByRole("button", { name: "New activity ↓" }),
      ).toBeInTheDocument();
    });

    it("jumps to the bottom from the pill and hides it", async () => {
      const user = userEvent.setup();
      render(<AssistantPane />);
      const top = stubLog(logOf(), 100);
      fireEvent.scroll(logOf());
      act(() =>
        useAssistantStore.getState().apply({ type: "message", text: "more" }),
      );
      await user.click(screen.getByRole("button", { name: "New activity ↓" }));
      expect(top()).toBe(1000);
      expect(
        screen.queryByRole("button", { name: "New activity ↓" }),
      ).not.toBeInTheDocument();
    });

    it("hides the pill when the user scrolls to the bottom by hand", () => {
      render(<AssistantPane />);
      const log = logOf();
      stubLog(log, 100);
      fireEvent.scroll(log);
      act(() =>
        useAssistantStore.getState().apply({ type: "message", text: "more" }),
      );
      stubLog(log, 700);
      fireEvent.scroll(log);
      expect(
        screen.queryByRole("button", { name: "New activity ↓" }),
      ).not.toBeInTheDocument();
    });

    it("follows the tail while at the bottom", () => {
      render(<AssistantPane />);
      const top = stubLog(logOf(), 700);
      fireEvent.scroll(logOf());
      act(() =>
        useAssistantStore.getState().apply({ type: "message", text: "more" }),
      );
      expect(top()).toBe(1000);
    });

    it("shows no pill when only the busy flag changes", () => {
      render(<AssistantPane />);
      stubLog(logOf(), 100);
      fireEvent.scroll(logOf());
      act(() => useAssistantStore.setState({ busy: true }));
      expect(
        screen.queryByRole("button", { name: "New activity ↓" }),
      ).not.toBeInTheDocument();
    });

    it("keeps a pinned reader at the bottom when a folded run opens", () => {
      render(<AssistantPane />);
      act(() =>
        useAssistantStore.getState().apply({ type: "tool", name: "Agent" }),
      );
      const log = logOf();
      const top = stubLog(log, 700);
      fireEvent.scroll(log);
      // The list grows below the viewport: no scroll event, just a toggle.
      fireEvent(
        document.querySelector("details") as HTMLDetailsElement,
        new Event("toggle"),
      );
      expect(top()).toBe(1000);
      // Still following: the next reply is not announced by a pill.
      act(() =>
        useAssistantStore.getState().apply({ type: "message", text: "more" }),
      );
      expect(
        screen.queryByRole("button", { name: "New activity ↓" }),
      ).not.toBeInTheDocument();
    });

    it("leaves a reader who scrolled up alone when a run opens", () => {
      render(<AssistantPane />);
      act(() =>
        useAssistantStore.getState().apply({ type: "tool", name: "Agent" }),
      );
      const log = logOf();
      const top = stubLog(log, 100);
      fireEvent.scroll(log);
      fireEvent(
        document.querySelector("details") as HTMLDetailsElement,
        new Event("toggle"),
      );
      expect(top()).toBe(100);
    });

    it("does not scroll down when an earlier run opens", () => {
      render(<AssistantPane />);
      act(() => {
        const { apply } = useAssistantStore.getState();
        apply({ type: "tool", name: "Agent" });
        apply({ type: "message", text: "between" });
        apply({ type: "tool", name: "Agent" });
      });
      const log = logOf();
      const top = stubLog(log, 700);
      fireEvent.scroll(log);
      fireEvent(
        document.querySelectorAll("details")[0] as HTMLDetailsElement,
        new Event("toggle"),
      );
      expect(top()).toBe(700);
    });

    it("re-pins to the bottom when the transcript shrinks", () => {
      render(<AssistantPane />);
      act(() =>
        useAssistantStore.getState().apply({ type: "message", text: "hi" }),
      );
      const log = logOf();
      const top = stubLog(log, 100);
      fireEvent.scroll(log);
      act(() => useAssistantStore.setState({ messages: [] }));
      expect(top()).toBe(1000);
      act(() =>
        useAssistantStore.getState().apply({ type: "message", text: "new" }),
      );
      expect(
        screen.queryByRole("button", { name: "New activity ↓" }),
      ).not.toBeInTheDocument();
    });

    it("offers no pill for new actions or nudges alone", () => {
      render(<AssistantPane />);
      stubLog(logOf(), 100);
      fireEvent.scroll(logOf());
      act(() => {
        const { apply } = useAssistantStore.getState();
        apply({ type: "tool", name: "Agent" });
        apply({ type: "toolFailed", message: "boom" });
        apply({ type: "notice", text: "[Podium] sweep" });
      });
      expect(
        screen.queryByRole("button", { name: "New activity ↓" }),
      ).not.toBeInTheDocument();
    });

    it("clears the pill when the transcript is emptied", () => {
      render(<AssistantPane />);
      stubLog(logOf(), 100);
      fireEvent.scroll(logOf());
      act(() =>
        useAssistantStore.getState().apply({ type: "message", text: "more" }),
      );
      expect(
        screen.getByRole("button", { name: "New activity ↓" }),
      ).toBeInTheDocument();
      act(() => useAssistantStore.setState({ messages: [] }));
      expect(
        screen.queryByRole("button", { name: "New activity ↓" }),
      ).not.toBeInTheDocument();
    });

    it("'Start my day' jumps to the bottom directly", async () => {
      const user = userEvent.setup();
      render(<AssistantPane />);
      const top = stubLog(logOf(), 100);
      fireEvent.scroll(logOf());
      await user.click(screen.getByLabelText("Start my day"));
      expect(top()).toBe(1000);
    });

    it("sending a message always jumps to the bottom", async () => {
      const user = userEvent.setup();
      render(<AssistantPane />);
      const top = stubLog(logOf(), 100);
      fireEvent.scroll(logOf());
      await user.type(screen.getByLabelText("Message the assistant"), "hi");
      await user.click(screen.getByLabelText("Send"));
      expect(top()).toBe(1000);
    });
  });

  it("reports a failed send in the log instead of losing it", async () => {
    vi.mocked(assistantSend).mockRejectedValueOnce({
      kind: "invalidInput",
      message: "message must not be blank",
    });
    const user = userEvent.setup();
    render(<AssistantPane />);

    await user.type(screen.getByLabelText("Message the assistant"), "hi");
    await user.click(screen.getByLabelText("Send"));

    expect(screen.getByText("message must not be blank")).toBeInTheDocument();
  });

  /** Stream in one pending proposal card. */
  function streamProposal() {
    act(() => {
      useAssistantStore.getState().apply({
        type: "proposal",
        id: "p-1",
        tool: "set_todo_archived",
        summary: '{"todoId":"t-1"}',
      });
    });
  }

  it("confirms a proposed action", async () => {
    const user = userEvent.setup();
    render(<AssistantPane />);
    streamProposal();

    expect(screen.getByText("set todo archived")).toBeInTheDocument();
    expect(screen.getByText('{"todoId":"t-1"}')).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Confirm" }));

    expect(assistantResolveProposal).toHaveBeenCalledWith("p-1", true);
    // The buttons are gone: one decision per proposal.
    expect(
      screen.queryByRole("button", { name: "Confirm" }),
    ).not.toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("thinking…");
  });

  it("rejects a proposed action and shows the outcome", async () => {
    const user = userEvent.setup();
    render(<AssistantPane />);
    streamProposal();

    await user.click(screen.getByRole("button", { name: "Reject" }));

    expect(assistantResolveProposal).toHaveBeenCalledWith("p-1", false);
    expect(screen.getByText("Rejected")).toBeInTheDocument();
  });

  it("reports a confirmed action that failed", () => {
    render(<AssistantPane />);
    streamProposal();

    act(() => {
      useAssistantStore.getState().apply({
        type: "proposalResolved",
        id: "p-1",
        approved: true,
        error: "to-do not found",
      });
    });

    expect(screen.getByText("Failed: to-do not found")).toBeInTheDocument();
  });

  it("loads and changes the autonomy mode", async () => {
    vi.mocked(assistantAutonomyGet).mockResolvedValue("propose-all");
    const user = userEvent.setup();
    render(<AssistantPane />);

    await vi.waitFor(() =>
      expect(screen.getByLabelText("Autonomy")).toHaveValue("propose-all"),
    );

    await user.selectOptions(screen.getByLabelText("Autonomy"), "full");

    expect(assistantAutonomySet).toHaveBeenCalledWith("full");
    expect(useAssistantStore.getState().autonomy).toBe("full");
  });

  it("hides itself via the header button", async () => {
    const user = userEvent.setup();
    useLayoutStore.setState({ assistantOpen: true });
    render(<AssistantPane />);

    await user.click(screen.getByLabelText("Hide assistant"));

    expect(useLayoutStore.getState().assistantOpen).toBe(false);
  });

  describe("persistence and memory", () => {
    it("replays the stored transcript into the log on mount", async () => {
      vi.mocked(assistantTranscript).mockResolvedValue([
        { role: "user", text: "what is open?", at: "2026-01-02T09:00:00Z" },
        { role: "assistant", text: "Two to-dos.", at: "2026-01-02T09:00:01Z" },
        { role: "tool", text: "list_todos", at: "2026-01-02T09:00:02Z" },
      ]);

      render(<AssistantPane />);

      expect(await screen.findByText("what is open?")).toBeInTheDocument();
      expect(screen.getByText("Two to-dos.")).toBeInTheDocument();
      expect(screen.getByText("Ran 1 action · list todos")).toBeInTheDocument();
    });

    it("demotes a replayed nudge just like a live one", async () => {
      vi.mocked(assistantTranscript).mockResolvedValue([
        {
          role: "nudge",
          text: '[Podium] The agent "docs" finished (exit code 0). Check what it did and tell the user.',
          at: "2026-01-02T09:00:00Z",
        },
      ]);

      render(<AssistantPane />);

      expect(
        await screen.findByText("Podium nudged the assistant"),
      ).toBeInTheDocument();
      expect(screen.queryByText(/Check what it did/)).not.toBeInTheDocument();
    });

    it("offers no new-conversation control — resetting lives in Settings", async () => {
      vi.mocked(assistantTranscript).mockResolvedValue([
        { role: "user", text: "old turn", at: "2026-01-02T09:00:00Z" },
      ]);
      render(<AssistantPane />);
      expect(await screen.findByText("old turn")).toBeInTheDocument();

      expect(screen.queryByLabelText("New conversation")).toBeNull();
      expect(assistantNewConversation).not.toHaveBeenCalled();
    });

    it("shows the stored memory and saves an edit", async () => {
      const user = userEvent.setup();
      vi.mocked(assistantMemoryGet).mockResolvedValue("Prefers short answers.");
      render(<AssistantPane />);

      await user.click(screen.getByLabelText("Assistant memory"));
      const box = await screen.findByLabelText("Memory");
      expect(box).toHaveValue("Prefers short answers.");

      await user.type(box, " Ships on Fridays.");
      await user.click(screen.getByRole("button", { name: "Save" }));

      expect(assistantMemorySet).toHaveBeenCalledWith(
        "Prefers short answers. Ships on Fridays.",
      );
      // Saving closes the view again.
      expect(screen.queryByLabelText("Memory")).not.toBeInTheDocument();
    });

    it("clears the memory box, then persists the clear on save", async () => {
      const user = userEvent.setup();
      vi.mocked(assistantMemoryGet).mockResolvedValue("Something old.");
      render(<AssistantPane />);

      await user.click(screen.getByLabelText("Assistant memory"));
      expect(await screen.findByLabelText("Memory")).toHaveValue(
        "Something old.",
      );

      await user.click(screen.getByRole("button", { name: "Clear" }));
      expect(screen.getByLabelText("Memory")).toHaveValue("");

      await user.click(screen.getByRole("button", { name: "Save" }));
      expect(assistantMemorySet).toHaveBeenCalledWith("");
    });
  });

  describe("the pending queue", () => {
    // jsdom does no layout, so this pins the rule itself. The tail is a
    // scroll container inside the `.log` flex column; without
    // `flex-shrink: 0` its automatic min-height is 0 and a long transcript
    // squeezes it to nothing, hiding every waiting message.
    it("never lets the transcript squeeze the tail away", () => {
      const tail = /(?:^|\n)\.tail\s*\{([^}]*)\}/.exec(css)?.[1] ?? "";
      expect(tail).toMatch(/overflow-y:\s*auto/);
      expect(tail).toMatch(/flex-shrink:\s*0/);
    });

    it("shows nothing when the queue is empty", () => {
      render(<AssistantPane />);
      expect(screen.queryByLabelText("Edit waiting message")).toBeNull();
      expect(screen.queryByLabelText("Remove waiting message")).toBeNull();
    });

    it("renders a waiting user message with edit/remove controls", () => {
      useAssistantStore.setState({
        pending: [
          {
            id: "p-1",
            kind: "user",
            text: "queued while busy",
            attachments: [],
            replyTo: null,
          },
        ],
      });
      render(<AssistantPane />);

      expect(screen.getByText("queued while busy")).toBeInTheDocument();
      expect(screen.getByText("waiting…")).toBeInTheDocument();
      expect(screen.getByLabelText("Edit waiting message")).toBeInTheDocument();
      expect(
        screen.getByLabelText("Remove waiting message"),
      ).toBeInTheDocument();
    });

    it("prefixes a waiting reply with the item it answers", () => {
      useAssistantStore.setState({
        pending: [
          {
            id: "p-1",
            kind: "user",
            text: "Drop it",
            attachments: [],
            replyTo: "Checkout fix · Webshop",
          },
        ],
      });
      render(<AssistantPane />);

      expect(
        screen.getByText(/Re: Checkout fix · Webshop/),
      ).toBeInTheDocument();
      expect(screen.getByText("Drop it")).toBeInTheDocument();
    });

    it("does not offer edit/remove for a nudge or notice waiting its turn", () => {
      useAssistantStore.setState({
        pending: [
          {
            id: "n-1",
            kind: "nudge",
            text: "[Podium] an agent finished",
            attachments: [],
            replyTo: null,
          },
        ],
      });
      render(<AssistantPane />);

      expect(
        screen.getByText("[Podium] an agent finished"),
      ).toBeInTheDocument();
      expect(screen.queryByLabelText("Edit waiting message")).toBeNull();
      expect(screen.queryByLabelText("Remove waiting message")).toBeNull();
    });

    it("edits a waiting message on Enter", async () => {
      const user = userEvent.setup();
      useAssistantStore.setState({
        pending: [
          {
            id: "p-1",
            kind: "user",
            text: "first draft",
            attachments: [],
            replyTo: null,
          },
        ],
      });
      render(<AssistantPane />);

      await user.click(screen.getByLabelText("Edit waiting message"));
      const input = screen.getByLabelText("Edit waiting message");
      await user.clear(input);
      await user.type(input, "final draft{Enter}");

      expect(assistantQueueEdit).toHaveBeenCalledWith("p-1", "final draft");
    });

    it("sends a waiting message now", async () => {
      const user = userEvent.setup();
      useAssistantStore.setState({
        pending: [
          {
            id: "p-1",
            kind: "user",
            text: "send me now",
            attachments: [],
            replyTo: null,
          },
        ],
      });
      render(<AssistantPane />);

      await user.click(screen.getByLabelText("Send waiting message now"));

      expect(assistantQueueSendNow).toHaveBeenCalledWith("p-1");
    });

    it("removes a waiting message", async () => {
      const user = userEvent.setup();
      useAssistantStore.setState({
        pending: [
          {
            id: "p-1",
            kind: "user",
            text: "withdraw me",
            attachments: [],
            replyTo: null,
          },
        ],
      });
      render(<AssistantPane />);

      await user.click(screen.getByLabelText("Remove waiting message"));

      expect(assistantQueueRemove).toHaveBeenCalledWith("p-1");
    });

    it("offers an interrupt control only while a turn is running", async () => {
      const user = userEvent.setup();
      const { rerender } = render(<AssistantPane />);
      expect(screen.queryByLabelText("Interrupt")).toBeNull();

      useAssistantStore.setState({ busy: true });
      rerender(<AssistantPane />);
      await user.click(screen.getByLabelText("Interrupt"));

      expect(assistantInterrupt).toHaveBeenCalled();
    });

    // jsdom has no layout, so these assert the *structure* the pinning needs
    // (one container, busy row first), never actual stickiness or pixels.
    it("keeps the waiting rows below the busy indicator, in one pinned row", () => {
      useAssistantStore.setState({
        busy: true,
        pending: [
          {
            id: "p-1",
            kind: "user",
            text: "queued while busy",
            attachments: [],
            replyTo: null,
          },
        ],
      });
      const { container } = render(<AssistantPane />);

      const tail = container.querySelector(`.${styles.tail}`);
      const busyRow = screen.getByRole("status");
      const waiting = screen.getByText("queued while busy");

      expect(tail).not.toBeNull();
      expect(tail).toContainElement(busyRow);
      expect(tail).toContainElement(waiting);
      expect(
        busyRow.compareDocumentPosition(waiting) &
          Node.DOCUMENT_POSITION_FOLLOWING,
      ).toBeTruthy();
    });

    it("renders no pinned row when idle with an empty queue", () => {
      const { container } = render(<AssistantPane />);
      expect(container.querySelector(`.${styles.tail}`)).toBeNull();
    });
  });

  describe("composer history recall", () => {
    function seedSentMessages() {
      useAssistantStore.setState({
        messages: [
          { id: 1, role: "user", text: "first message" },
          { id: 2, role: "assistant", text: "an answer" },
          { id: 3, role: "user", text: "second message" },
        ],
      });
    }

    it("arrow-up recalls the previous sent messages, newest first", async () => {
      seedSentMessages();
      const user = userEvent.setup();
      render(<AssistantPane />);
      const input = screen.getByLabelText(
        "Message the assistant",
      ) as HTMLTextAreaElement;
      input.focus();

      await user.keyboard("{ArrowUp}");
      expect(input).toHaveValue("second message");

      await user.keyboard("{ArrowUp}");
      expect(input).toHaveValue("first message");

      // Already at the oldest entry — stays put rather than wrapping.
      await user.keyboard("{ArrowUp}");
      expect(input).toHaveValue("first message");
    });

    it("arrow-down walks forward and restores the draft past the newest entry", async () => {
      seedSentMessages();
      const user = userEvent.setup();
      render(<AssistantPane />);
      const input = screen.getByLabelText(
        "Message the assistant",
      ) as HTMLTextAreaElement;
      input.focus();
      await user.type(input, "my unsent draft");

      await user.keyboard("{ArrowUp}");
      expect(input).toHaveValue("second message");
      await user.keyboard("{ArrowUp}");
      expect(input).toHaveValue("first message");

      await user.keyboard("{ArrowDown}");
      expect(input).toHaveValue("second message");

      // Past the newest recalled entry: back to what was being typed.
      await user.keyboard("{ArrowDown}");
      expect(input).toHaveValue("my unsent draft");
    });

    it("does not recall when the caret isn't on the first/last line", async () => {
      seedSentMessages();
      const user = userEvent.setup();
      render(<AssistantPane />);
      const input = screen.getByLabelText(
        "Message the assistant",
      ) as HTMLTextAreaElement;

      await user.type(input, "line one{Shift>}{Enter}{/Shift}line two");
      // Caret sits at the end, on the second line — arrow-up should move the
      // cursor within the text, not recall history.
      await user.keyboard("{ArrowUp}");
      expect(input).toHaveValue("line one\nline two");

      // Move the caret to the very start (first line) and try again: now it
      // recalls.
      input.setSelectionRange(0, 0);
      await user.keyboard("{ArrowUp}");
      expect(input).toHaveValue("second message");
    });

    it("does not send while recalling, and sending a recalled+edited message leaves history intact", async () => {
      seedSentMessages();
      const user = userEvent.setup();
      render(<AssistantPane />);
      const input = screen.getByLabelText(
        "Message the assistant",
      ) as HTMLTextAreaElement;
      input.focus();

      await user.keyboard("{ArrowUp}");
      expect(assistantSend).not.toHaveBeenCalled();
      expect(input).toHaveValue("second message");

      await user.type(input, " edited");
      await user.click(screen.getByLabelText("Send"));
      expect(assistantSend).toHaveBeenCalledWith("second message edited", []);
      // Sending moved focus to the Send button; bring it back to the
      // composer before recalling again.
      input.focus();

      // Recalling again finds the sent message as a new, separate entry —
      // the original "second message" is still there behind it.
      await user.keyboard("{ArrowUp}");
      expect(input).toHaveValue("second message edited");
      await user.keyboard("{ArrowUp}");
      expect(input).toHaveValue("second message");
    });

    it("does nothing when there is no sent history", async () => {
      const user = userEvent.setup();
      render(<AssistantPane />);
      const input = screen.getByLabelText(
        "Message the assistant",
      ) as HTMLTextAreaElement;
      input.focus();
      await user.type(input, "typing");

      await user.keyboard("{ArrowUp}");
      expect(input).toHaveValue("typing");
    });
  });

  describe("day start", () => {
    it("shows the day-start button in the empty state and triggers it", async () => {
      const user = userEvent.setup();
      render(<AssistantPane />);

      await user.click(screen.getByLabelText("Start my day"));

      expect(assistantDayStart).toHaveBeenCalled();
      // The turn shows at once, so the button is gone before the CLI answers
      // — no window in which the click looks like it did nothing.
      expect(screen.queryByLabelText("Start my day")).toBeNull();
      expect(screen.getByText("Start my day")).toBeInTheDocument();
      expect(screen.getByText("thinking…")).toBeInTheDocument();
    });

    it("hides the day-start button once there are messages", () => {
      useAssistantStore.setState({
        messages: [{ id: 1, role: "user", text: "hi" }],
      });
      render(<AssistantPane />);

      expect(screen.queryByLabelText("Start my day")).toBeNull();
    });
  });

  describe("attachments", () => {
    it("attaches picked files as absolute paths and sends them with the turn", async () => {
      const user = userEvent.setup();
      vi.mocked(openFileDialog).mockResolvedValue([
        "/abs/notes.txt",
      ] as unknown as string);
      render(<AssistantPane />);

      await user.click(screen.getByLabelText("Attach files"));
      expect(screen.getByText("notes.txt")).toBeInTheDocument();

      await user.type(
        screen.getByLabelText("Message the assistant"),
        "read it",
      );
      await user.click(screen.getByLabelText("Send"));

      expect(assistantSend).toHaveBeenCalledWith("read it", [
        { kind: "path", path: "/abs/notes.txt" },
      ]);
      // The tray clears with the composer.
      expect(screen.queryByText("notes.txt")).toBeNull();
    });

    it("removes an attachment before it is sent", async () => {
      const user = userEvent.setup();
      vi.mocked(openFileDialog).mockResolvedValue([
        "/abs/notes.txt",
      ] as unknown as string);
      render(<AssistantPane />);

      await user.click(screen.getByLabelText("Attach files"));
      await user.click(screen.getByLabelText("Remove notes.txt"));

      expect(screen.queryByText("notes.txt")).toBeNull();
      // Nothing left to send, so the button goes back to disabled.
      expect(screen.getByLabelText("Send")).toBeDisabled();
    });

    it("sends an attachment with no typed message", async () => {
      const user = userEvent.setup();
      vi.mocked(openFileDialog).mockResolvedValue([
        "/abs/shot.png",
      ] as unknown as string);
      render(<AssistantPane />);

      await user.click(screen.getByLabelText("Attach files"));
      expect(screen.getByLabelText("Send")).toBeEnabled();
      await user.click(screen.getByLabelText("Send"));

      expect(assistantSend).toHaveBeenCalledWith("(sent without a message)", [
        { kind: "path", path: "/abs/shot.png" },
      ]);
    });

    it("refuses more than five attachments, naming each one it dropped", async () => {
      const user = userEvent.setup();
      vi.mocked(openFileDialog).mockResolvedValue(
        Array.from(
          { length: 7 },
          (_, i) => `/abs/f${i}.txt`,
        ) as unknown as string,
      );
      render(<AssistantPane />);

      await user.click(screen.getByLabelText("Attach files"));

      expect(screen.getByText("f4.txt")).toBeInTheDocument();
      expect(screen.queryByText("f5.txt")).toBeNull();
      // The two over the cap are reported, not silently dropped.
      expect(screen.getByText(/"f5.txt" was not added/)).toBeInTheDocument();
      expect(screen.getByText(/"f6.txt" was not added/)).toBeInTheDocument();
    });

    it("attaches a pasted image as inline base64", async () => {
      render(<AssistantPane />);
      const input = screen.getByLabelText("Message the assistant");
      const file = new File([new Uint8Array([1, 2, 3])], "shot.png", {
        type: "image/png",
      });

      await act(async () => {
        fireEvent.paste(input, { clipboardData: { files: [file] } });
      });

      expect(await screen.findByText("shot.png")).toBeInTheDocument();
      await act(async () => {
        fireEvent.click(screen.getByLabelText("Send"));
      });
      expect(assistantSend).toHaveBeenCalledWith("(sent without a message)", [
        {
          kind: "image",
          name: "shot.png",
          mediaType: "image/png",
          data: "AQID",
        },
      ]);
    });

    it("attaches a dropped text file inline, and reports one it cannot take", async () => {
      render(<AssistantPane />);
      const pane = screen.getByLabelText("Personal assistant");
      const text = new File(["log line"], "run.log", { type: "text/plain" });
      const binary = new File([new Uint8Array([0, 1])], "archive.zip", {
        type: "application/zip",
      });

      await act(async () => {
        fireEvent.drop(pane, { dataTransfer: { files: [text, binary] } });
      });

      expect(await screen.findByText("run.log")).toBeInTheDocument();
      // A dropped file carries bytes but no path, so a binary one has no way
      // through — it is named rather than vanishing.
      expect(
        screen.getByText(/"archive.zip" was not added/),
      ).toBeInTheDocument();
    });
  });

  it("shows a calm off state without the composer, and Turn on restores the panel", async () => {
    useAssistantStore.setState({ enabled: false });
    const user = userEvent.setup();
    render(<AssistantPane />);

    expect(screen.getByText("The assistant is off")).toBeInTheDocument();
    expect(screen.queryByPlaceholderText("Message the assistant…")).toBeNull();
    expect(screen.queryByLabelText("Start my day")).toBeNull();

    await user.click(screen.getByRole("button", { name: "Turn on" }));
    expect(assistantEnabledSet).toHaveBeenCalledWith(true);
    expect(
      await screen.findByPlaceholderText("Message the assistant…"),
    ).toBeInTheDocument();
    expect(screen.queryByText("The assistant is off")).toBeNull();
  });
});
