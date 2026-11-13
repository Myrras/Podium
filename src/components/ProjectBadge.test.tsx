import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { ProjectBadge } from "./ProjectBadge";

describe("ProjectBadge", () => {
  it("shows initials in the project colour at the given size", () => {
    const { container } = render(
      <ProjectBadge
        project={{ color: "#ffffff", iconInitials: "AB" }}
        size={40}
      />,
    );
    expect(screen.getByText("AB")).toBeInTheDocument();
    expect(container.firstChild).toHaveStyle({
      background: "#ffffff",
      color: "#000",
      width: "40px",
    });
  });
});
