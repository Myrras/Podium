import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

const { openUrlMock } = vi.hoisted(() => ({ openUrlMock: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: openUrlMock }));

import { Markdown } from "./Markdown";

describe("Markdown", () => {
  it("opens links via the OS default browser instead of navigating the webview", () => {
    openUrlMock.mockClear();
    render(
      <Markdown>{"See [the issue](https://example.com/issues/1)."}</Markdown>,
    );

    const anchor = screen.getByRole("link", { name: "the issue" });
    expect(anchor).toHaveAttribute("target", "_blank");
    expect(anchor).toHaveAttribute("rel", "noreferrer");

    const event = fireEvent.click(anchor);

    expect(event).toBe(false); // preventDefault() was called, so the webview never navigates
    expect(openUrlMock).toHaveBeenCalledWith("https://example.com/issues/1");
  });
});
