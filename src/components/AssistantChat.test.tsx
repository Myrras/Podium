import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(() => Promise.resolve(null)),
}));

import { useAssistantStore } from "../state/assistantStore";
import { AssistantChat } from "./AssistantChat";

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
});
