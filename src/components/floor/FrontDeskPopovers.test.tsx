import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => undefined)),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(() => Promise.resolve(null)),
}));
vi.mock("../../lib/floor/camera", () => ({ showAgent: vi.fn() }));
// The avatars only read a bot variant from the rooms.
vi.mock("../../lib/floor/rooms", () => ({
  useRooms: () => [],
  roomOf: () => null,
}));

import type { ResponseItem } from "../../ipc/types";
import { useAssistantStore } from "../../state/assistantStore";
import { useFloorStore } from "../../state/floorStore";
import { FrontDesk } from "./FrontDesk";
import { FrontDeskPopovers } from "./FrontDeskPopovers";

const initialAssistant = useAssistantStore.getState();

const question: ResponseItem = {
  id: "r-1",
  kind: "needsYou",
  text: "Which branch should the retry fix go on?",
  project: "Harbor",
  agent: "Agent 1",
  agentColor: "#3e63dd",
  todo: null,
  source: null,
  processId: "proc-1",
  todoId: null,
  createdAt: "2026-01-01T10:00:00Z",
  key: "agent:proc-1",
  fingerprint: "",
  state: "open",
  reason: "question",
  sender: null,
  label: "Agent 1 · Harbor",
};

function renderDesk() {
  const { toggleDesk } = useFloorStore.getState();
  return render(
    <>
      <FrontDesk
        onOpenChat={() => toggleDesk("chat")}
        onOpenList={() => toggleDesk("list")}
      />
      <FrontDeskPopovers />
    </>,
  );
}

beforeEach(() => {
  useFloorStore.getState().reset();
  useAssistantStore.setState(
    {
      ...initialAssistant,
      enabled: true,
      responses: [question],
      // Mount-time loads of the chat: no backend here.
      loadAutonomy: vi.fn(() => Promise.resolve()),
      hydrate: vi.fn(() => Promise.resolve()),
      loadResponses: vi.fn(() => Promise.resolve()),
    },
    true,
  );
});

const chat = () => screen.queryByLabelText("Message the assistant");
const list = () => screen.queryByRole("region", { name: "Responses" });

describe("FrontDeskPopovers", () => {
  it("opens the overview list from the count chip", async () => {
    const user = userEvent.setup();
    renderDesk();
    await user.click(screen.getByRole("button", { name: "1 needs you" }));
    expect(list()).toBeInTheDocument();
    expect(screen.getByText("What needs you")).toBeInTheDocument();
    expect(chat()).toBeNull();
    // Text selected in the list is not assistant context (lib/selection.ts).
    expect(list()!.closest("[data-assistant-pane]")).not.toBeNull();
  });

  it("opens the chat from the bot, with the bot's state line and controls", async () => {
    const user = userEvent.setup();
    renderDesk();
    await user.click(screen.getByRole("button", { name: "Open chat" }));
    expect(chat()).toBeInTheDocument();
    expect(screen.getByRole("heading")).toHaveTextContent("AssistantIdle");
    expect(screen.getByLabelText("Autonomy")).toBeInTheDocument();
    expect(screen.getByLabelText("Assistant memory")).toBeInTheDocument();
    expect(list()).toBeNull();
  });

  it("keeps one popover open at a time", async () => {
    const user = userEvent.setup();
    renderDesk();
    await user.click(screen.getByRole("button", { name: "Open chat" }));
    await user.click(screen.getByRole("button", { name: "1 needs you" }));
    expect(list()).toBeInTheDocument();
    expect(chat()).not.toBeVisible();

    // A fixture popover replaces the desk's.
    act(() =>
      useFloorStore
        .getState()
        .openPopover({ projectId: "proj-1", kind: "todos", itemId: null }),
    );
    expect(list()).toBeNull();
  });

  it("closes on Esc only with focus inside", async () => {
    const user = userEvent.setup();
    renderDesk();
    const bot = screen.getByRole("button", { name: "Open chat" });
    await user.click(bot);

    fireEvent.keyDown(screen.getByRole("button", { name: "1 needs you" }), {
      key: "Escape",
    });
    expect(chat()).toBeInTheDocument();

    fireEvent.keyDown(chat()!, { key: "Escape" });
    expect(chat()).not.toBeVisible();
    expect(useFloorStore.getState().deskPopover).toBeNull();
  });

  it("keeps the chat mounted while closed, hidden and inert, so reopening is instant", async () => {
    const user = userEvent.setup();
    renderDesk();
    const bot = screen.getByRole("button", { name: "Open chat" });
    await user.click(bot);
    const input = chat();
    expect(input).toBeVisible();

    fireEvent.keyDown(input!, { key: "Escape" });
    const popover = input!.closest<HTMLElement>('[role="group"]')!;
    expect(popover).not.toBeVisible();
    expect(popover).toHaveAttribute("inert");

    // The hidden chat's outside-click handler is off: a click in the list keeps it open.
    await user.click(screen.getByRole("button", { name: "1 needs you" }));
    fireEvent.mouseDown(list()!);
    expect(useFloorStore.getState().deskPopover).toBe("list");

    await user.click(bot);
    expect(chat()).toBe(input);
    expect(chat()).toBeVisible();
    expect(popover).not.toHaveAttribute("inert");
    // Mounted once: the chat's loads ran on the first open only.
    expect(useAssistantStore.getState().loadResponses).toHaveBeenCalledTimes(1);
    expect(useAssistantStore.getState().loadAutonomy).toHaveBeenCalledTimes(1);
  });

  it("takes over a handed-over draft in the chat", () => {
    renderDesk();
    act(() => {
      useAssistantStore.getState().setDraft("Start an agent in Harbor on ");
      useFloorStore.getState().openDesk("chat");
    });
    expect(chat()).toHaveValue("Start an agent in Harbor on ");
  });
});
