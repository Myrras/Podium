import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Bot, MiniBot, Monitor, TerminalDeskArt } from "./Bot";

describe("Bot", () => {
  it("sets state, size and a seed-derived phase", () => {
    const a = render(
      <Bot color="#58a6ff" state="working" seed="a1" size={48} />,
    ).container;
    const b = render(
      <Bot color="#58a6ff" state="working" seed="zz9" />,
    ).container;
    const bot = a.firstElementChild as HTMLElement;
    expect(bot.dataset.state).toBe("working");
    expect(bot.style.getPropertyValue("--s")).toBe("48px");
    const ph = (e: Element) =>
      (e as HTMLElement).style.getPropertyValue("--phase");
    const pa = ph(bot.firstElementChild!);
    expect(Number(pa)).toBeGreaterThanOrEqual(0);
    expect(Number(pa)).toBeLessThan(1);
    expect(pa).not.toBe(ph(b.firstElementChild!.firstElementChild!));
  });

  it("spreads the phases of many bots over the beat", () => {
    const phases = Array.from({ length: 40 }, (_, i) => {
      const c = render(
        <Bot color="#58a6ff" state="working" seed={`proc-${i}-9f2c`} />,
      ).container;
      return Number(
        (
          c.firstElementChild!.firstElementChild as HTMLElement
        ).style.getPropertyValue("--phase"),
      );
    });
    expect(new Set(phases).size).toBe(40);
    expect(Math.min(...phases)).toBeLessThan(0.25);
    expect(Math.max(...phases)).toBeGreaterThan(0.75);
  });

  it("greys ended and idle bots and falls back without a colour", () => {
    const img = (c: Element) =>
      (c.firstElementChild!.firstElementChild as HTMLElement).style
        .backgroundImage;
    const ended = render(
      <Bot color="#58a6ff" state="ended" seed="s" />,
    ).container;
    const idle = render(
      <Bot color="#58a6ff" state="idle" seed="s" />,
    ).container;
    const none = render(
      <Bot color={null} state="working" seed="s" />,
    ).container;
    expect(img(ended)).toContain(encodeURIComponent("#8b949e"));
    expect(img(idle)).toBe(img(ended));
    expect(img(none)).toContain(encodeURIComponent("#6e7681"));
  });

  it("draws the given design", () => {
    const img = (variant?: number) =>
      (
        render(
          <Bot color="#58a6ff" state="working" seed="s" variant={variant} />,
        ).container.firstElementChild!.firstElementChild as HTMLElement
      ).style.backgroundImage;
    expect(img(4)).not.toBe(img());
    expect(img(0)).toBe(img());
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
