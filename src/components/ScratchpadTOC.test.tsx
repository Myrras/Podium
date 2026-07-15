import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { Heading } from "../lib/tiptapHeadings";
import { ScratchpadTOC } from "./ScratchpadTOC";

const HEADINGS: Heading[] = [
  { level: 2, text: "First section", pos: 1 },
  { level: 3, text: "First subsection", pos: 20 },
  { level: 3, text: "Second subsection", pos: 40 },
  { level: 2, text: "Second section", pos: 60 },
];

describe("ScratchpadTOC", () => {
  it("renders an empty state when there are no headings", () => {
    render(<ScratchpadTOC headings={[]} onSelectHeading={vi.fn()} />);
    expect(screen.getByText("No headings yet")).toBeInTheDocument();
  });

  it("renders headings nested by level, in document order", () => {
    render(<ScratchpadTOC headings={HEADINGS} onSelectHeading={vi.fn()} />);

    const items = screen.getAllByRole("listitem");
    expect(items).toHaveLength(4);
    expect(items.map((li) => li.getAttribute("data-level"))).toEqual([
      "2",
      "3",
      "3",
      "2",
    ]);
    expect(items.map((li) => li.textContent)).toEqual([
      "First section",
      "First subsection",
      "Second subsection",
      "Second section",
    ]);
  });

  it("renders inside a scrollable container", () => {
    render(<ScratchpadTOC headings={HEADINGS} onSelectHeading={vi.fn()} />);
    const nav = screen.getByRole("navigation", { name: "On this page" });
    // The scroll container is a direct child of the nav, distinct from the
    // (non-scrolling) title above it — its own overflow-y: auto lives in
    // ScratchpadTOC.module.css.
    expect(nav.querySelector("ul")?.parentElement).not.toBeNull();
  });

  it("stays usable with 30+ headings (renders every entry)", () => {
    const many: Heading[] = Array.from({ length: 40 }, (_, i) => ({
      level: 2,
      text: `Heading ${i}`,
      pos: i * 10 + 1,
    }));
    render(<ScratchpadTOC headings={many} onSelectHeading={vi.fn()} />);
    expect(screen.getAllByRole("listitem")).toHaveLength(40);
  });

  it("renders H1 unindented, at the same level as H2", () => {
    const withTitle: Heading[] = [
      { level: 1, text: "Document Title", pos: 1 },
      { level: 2, text: "First section", pos: 20 },
    ];
    render(<ScratchpadTOC headings={withTitle} onSelectHeading={vi.fn()} />);

    const items = screen.getAllByRole("listitem");
    expect(items.map((li) => li.getAttribute("data-level"))).toEqual([
      "1",
      "2",
    ]);
    // No indent rule targets data-level="1" (same as "2") in
    // ScratchpadTOC.module.css — only "3" gets padding-left.
    expect(items[0].className).toBe(items[1].className);
  });

  it("calls onSelectHeading with the clicked heading", () => {
    const onSelectHeading = vi.fn();
    render(
      <ScratchpadTOC headings={HEADINGS} onSelectHeading={onSelectHeading} />,
    );

    fireEvent.click(screen.getByText("Second subsection"));

    expect(onSelectHeading).toHaveBeenCalledWith(HEADINGS[2]);
  });
});
