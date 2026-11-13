/** Renders the one open popover at its fixture (`data-fixture="<projectId>:<kind>"`). */

import { useEffect } from "react";

import { useFloorStore } from "../../state/floorStore";
import { useProjectStore } from "../../state/projectStore";
import { ProjectMenuPopover } from "./ProjectMenuPopover";
import { ScratchpadFixturePopover } from "./ScratchpadFixturePopover";
import { ServicesFixturePopover } from "./ServicesFixturePopover";
import { TodoFixturePopover } from "./TodoFixturePopover";

const CONTENT = {
  todos: TodoFixturePopover,
  scratchpads: ScratchpadFixturePopover,
  services: ServicesFixturePopover,
  menu: ProjectMenuPopover,
};

export function FloorPopovers() {
  const popover = useFloorStore((s) => s.popover);
  const closePopover = useFloorStore((s) => s.closePopover);
  const known = useProjectStore((s) =>
    popover ? s.projects.some((p) => p.id === popover.projectId) : false,
  );
  const anchor = popover
    ? document.querySelector<HTMLElement>(
        `[data-fixture="${popover.projectId}:${popover.kind}"]`,
      )
    : null;
  const open = popover !== null && known && anchor !== null;

  useEffect(() => {
    if (popover && !open) closePopover();
  }, [popover, open, closePopover]);

  if (!popover || !open) return null;
  const Content = CONTENT[popover.kind];
  return (
    <Content
      key={`${popover.projectId}:${popover.kind}`}
      projectId={popover.projectId}
      itemId={popover.itemId}
      anchor={anchor}
    />
  );
}
