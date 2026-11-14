/** Right-click plumbing: `open` shows a `ContextMenu` at the pointer, `element` renders it. */

import { useState } from "react";
import type { MouseEvent } from "react";

import { ContextMenu } from "../components/ContextMenu";
import type { MenuItem } from "../components/ContextMenu";

export function useContextMenu() {
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
    <ContextMenu {...menu} onClose={() => setMenu(null)} />
  );
  return { open, element };
}
