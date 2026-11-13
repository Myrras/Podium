import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Bot, MiniBot, Monitor, TerminalDeskArt } from "./Bot";

describe("Bot", () => {
  it("sets state, size and a seed-derived delay", () => {
    const a = render(
      <Bot color="#58a6ff" state="working" seed="a1" size={48} />,
    ).container;
    const b = render(
      <Bot color="#58a6ff" state="working" seed="zz9" />,
    ).container;
    const bot = a.firstElementChild as HTMLElement;
    expect(bot.dataset.state).toBe("working");
    expect(bot.style.getPropertyValue("--s")).toBe("48px");
    const da = (bot.firstElementChild as HTMLElement).style.animationDelay;
    expect(da).toMatch(/^-\d+ms$/);
    expect(da).not.toBe(
      (b.firstElementChild!.firstElementChild as HTMLElement).style
        .animationDelay,
    );
  });

  it("greys ended bots and falls back without a colour", () => {
    const img = (c: Element) =>
      (c.firstElementChild!.firstElementChild as HTMLElement).style
        .backgroundImage;
    const ended = render(
      <Bot color="#58a6ff" state="ended" seed="s" />,
    ).container;
    const none = render(<Bot color={null} state="idle" seed="s" />).container;
    expect(img(ended)).toContain(encodeURIComponent("#8b949e"));
    expect(img(none)).toContain(encodeURIComponent("#6e7681"));
  });

  it("renders mini bot, monitor and terminal desk", () => {
    const { container } = render(
      <>
        <MiniBot color={null} seed="m" />
        <Monitor seed="m" />
        <TerminalDeskArt />
      </>,
    );
    expect(container.querySelectorAll("i")).toHaveLength(3);
  });
});
