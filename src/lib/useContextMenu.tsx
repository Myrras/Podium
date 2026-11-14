/**
 * Right-click plumbing: `open` shows a `ContextMenu` at the pointer, `element`
 * renders it. `follow` closes it when what it sits on moves; inside a Popover
 * it defaults to the popover's own.
 */

import { useState } from "react";
import type { MouseEvent } from "react";

import { ContextMenu } from "../components/ContextMenu";
import type { MenuItem } from "../components/ContextMenu";
import { usePopoverFollow } from "../components/Popover";
import type { Follow } from "./viewport";

export function useContextMenu(follow?: Follow) {
  const inherited = usePopoverFollow();
  const [menu, setMenu] = useState<{
    x: number;
    y: number;
    items: MenuItem[];
  } | null>(null);
  // Stops propagation so an enclosing row or room does not open its own menu too.
  const open = (e: MouseEvent, items: MenuItem[]) => {
    e.preventDefault();
    e.stopPropagation();
    setMenu({ x: e.clientX, y: e.clientY, items });
  };
  const element = menu && (
    <ContextMenu
      {...menu}
      follow={follow ?? inherited}
      onClose={() => setMenu(null)}
    />
  );
  return { open, element };
}
