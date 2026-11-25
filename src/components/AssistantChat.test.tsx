import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(() => Promise.resolve(null)),
}));

import { useAssistantStore } from "../state/assistantStore";
import { AssistantChat } from "./AssistantChat";
import { Popover } from "./Popover";

const initial = useAssistantStore.getState();

function seed(enabled: boolean) {
  useAssistantStore.setState(
    {
      ...initial,
      enabled,
      // Mount-time loads: no backend here.
      loadAutonomy: vi.fn(() => Promise.resolve()),
      hydrate: vi.fn(() => Promise.resolve()),
      loadResponses: vi.fn(() => Promise.resolve()),
    },
    true,
  );
}

const renderChat = () =>
  render(
    <AssistantChat header={(controls) => <nav>{controls}</nav>}>
      <p>Host list</p>
    </AssistantChat>,
  );

describe("AssistantChat", () => {
  beforeEach(() => seed(true));

  it("hands its controls to the host's header and renders the host's children", () => {
    renderChat();
    const nav = screen.getByRole("navigation");
    expect(nav).toContainElement(screen.getByLabelText("Autonomy"));
    expect(nav).toContainElement(screen.getByLabelText("Assistant memory"));
    expect(screen.getByText("Host list")).toBeInTheDocument();
    expect(screen.getByLabelText("Message the assistant")).toBeInTheDocument();
  });

  it("shows the off state instead of children, log and composer", () => {
    seed(false);
    renderChat();
    expect(screen.getByText("The assistant is off")).toBeInTheDocument();
    expect(screen.queryByText("Host list")).toBeNull();
    expect(screen.queryByLabelText("Message the assistant")).toBeNull();
    expect(screen.getByLabelText("Autonomy")).toBeDisabled();
    expect(screen.queryByLabelText("Assistant memory")).toBeNull();
  });

  it("Esc cancels a waiting-message edit without saving or closing the popover", async () => {
    const user = userEvent.setup();
    const editPending = vi.fn(() => Promise.resolve());
    useAssistantStore.setState({
      editPending,
      pending: [
        { id: "q1", kind: "user", text: "first", attachments: [] },
      ] as never,
    });
    const anchor = document.createElement("button");
    document.body.append(anchor);
    const onClose = vi.fn();
    render(
      <Popover anchor={anchor} title="Chat" onClose={onClose}>
        <AssistantChat header={(c) => <nav>{c}</nav>}>
          <p>Host</p>
        </AssistantChat>
      </Popover>,
    );
    await user.click(
      screen.getByRole("button", { name: "Edit waiting message" }),
    );
    await user.type(
      screen.getByRole("textbox", { name: "Edit waiting message" }),
      " more{Escape}",
    );
    expect(screen.getByText("first")).toBeInTheDocument();
    expect(editPending).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
  });
});
