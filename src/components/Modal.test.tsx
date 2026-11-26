import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { Modal } from "./Modal";

describe("Modal", () => {
  it("renders outside its parent so clipped ancestors can't cut it off", () => {
    const { container } = render(
      <div style={{ overflow: "hidden" }}>
        <Modal open title="Hello" onClose={() => {}}>
          body
        </Modal>
      </div>,
    );
    const dialog = screen.getByRole("dialog", { name: "Hello" });
    expect(container.contains(dialog)).toBe(false);
    expect(document.body.contains(dialog)).toBe(true);
  });
});
