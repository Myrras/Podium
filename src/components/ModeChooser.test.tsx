import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";

import { useLayoutStore } from "../state/layoutStore";
import { ModeChooser } from "./ModeChooser";

beforeEach(() => {
  localStorage.clear();
  useLayoutStore.setState({ mode: null });
});

describe("ModeChooser", () => {
  it("stores the chosen mode", () => {
    render(<ModeChooser />);
    fireEvent.click(screen.getByRole("button", { name: /Floor mode/ }));
    expect(useLayoutStore.getState().mode).toBe("floor");
  });

  it("closing leaves the mode null and stays closed", async () => {
    render(<ModeChooser />);
    fireEvent.keyDown(document, { key: "Escape" });
    expect(useLayoutStore.getState().mode).toBeNull();
    await waitFor(() =>
      expect(
        screen.queryByRole("button", { name: /Project mode/ }),
      ).not.toBeInTheDocument(),
    );
  });

  it("does not show when a mode is stored", () => {
    useLayoutStore.setState({ mode: "project" });
    render(<ModeChooser />);
    expect(screen.queryByText("Choose your view")).not.toBeInTheDocument();
  });
});
