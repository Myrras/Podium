import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { CopyIdButton } from "./CopyIdButton";

describe("CopyIdButton", () => {
  it("copies the id to the clipboard and flashes confirmation", async () => {
    const writeText = vi.fn(() => Promise.resolve());
    Object.assign(navigator, { clipboard: { writeText } });

    render(<CopyIdButton id="todo-42" />);
    const btn = screen.getByRole("button", { name: "Copy id" });
    expect(btn.title).toBe("Copy id: todo-42");

    fireEvent.click(btn);
    expect(writeText).toHaveBeenCalledWith("todo-42");
    await waitFor(() => expect(btn.title).toBe("Copied!"));
  });
});
