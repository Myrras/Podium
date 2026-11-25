import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { AssistantDesk, deskLine } from "./AssistantDesk";

const pose = (c: HTMLElement) => c.querySelector("[data-pose]")!;

describe("AssistantDesk", () => {
  it.each([
    ["idle", 0, "Idle", "asleep"],
    ["answering", 0, "Answering", "typing"],
    ["answering", 2, "Looking things up with 2 helpers", "typing"],
    ["waiting", 0, "Waiting for your OK", "arm"],
    ["off", 0, "Off", "asleep"],
  ] as const)("%s with %i helpers", (state, helpers, line, p) => {
    const { container } = render(
      <AssistantDesk state={state} helpers={helpers} />,
    );
    expect(screen.getByText(line)).toBeInTheDocument();
    expect(pose(container)).toHaveAttribute("data-pose", p);
    expect(pose(container)).toHaveAttribute("data-state", state);
  });

  it("drives the bot's typing loop only while answering", () => {
    const { container, rerender } = render(
      <AssistantDesk state="answering" helpers={0} />,
    );
    expect(container.querySelector('[data-state="working"]')).not.toBeNull();
    rerender(<AssistantDesk state="idle" helpers={0} />);
    expect(container.querySelector('[data-state="working"]')).toBeNull();
  });

  it("caps helpers at 3 minis and then shows +N", () => {
    const { container } = render(
      <AssistantDesk state="answering" helpers={5} />,
    );
    expect(container.querySelectorAll("i").length).toBe(1 + 3);
    expect(screen.getByText("+2")).toBeInTheDocument();
  });

  it("shows no helper row without helpers", () => {
    render(<AssistantDesk state="idle" helpers={0} />);
    expect(screen.queryByTitle(/helpers/)).toBeNull();
  });

  it("leaves the state line to the host with hideLine", () => {
    render(<AssistantDesk state="waiting" helpers={0} hideLine />);
    expect(screen.queryByText("Waiting for your OK")).toBeNull();
  });

  it("uses the singular for one helper", () => {
    expect(deskLine("answering", 1)).toBe("Looking things up with 1 helper");
  });
});
