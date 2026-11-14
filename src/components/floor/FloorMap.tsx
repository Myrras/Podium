/**
 * The map: one transformed world layer with the rooms at their world rects.
 * The camera lives in floorStore and only touches the layer's style, never React state.
 */

import { useEffect, useRef } from "react";
import type { KeyboardEvent, PointerEvent } from "react";

import { jumpTo } from "../../lib/floor/camera";
import { FAR_Z, GRID, zoomAt } from "../../lib/floor/geometry";
import { useFloorData, useRooms } from "../../lib/floor/rooms";
import type { Camera } from "../../lib/floor/types";
import { useFloorStore } from "../../state/floorStore";
import { Room } from "./Room";
import styles from "./FloorMap.module.css";

const NO_PAN =
  'button, [role="button"], a, input, textarea, select, [data-no-pan]';
const PAN_THRESHOLD = 4;

export function FloorMap() {
  useFloorData();
  const rooms = useRooms();
  const viewportRef = useRef<HTMLDivElement>(null);
  const layerRef = useRef<HTMLDivElement>(null);
  const pan = useRef<{
    id: number;
    x: number;
    y: number;
    from: Camera;
    moving: boolean;
  } | null>(null);

  // Camera → layer transform, grid offset and far flag, outside React.
  useEffect(() => {
    const viewport = viewportRef.current!;
    const layer = layerRef.current!;
    const apply = ({ camera: { x, y, z }, animate }: typeof prev) => {
      layer.style.transform = `translate(${x}px, ${y}px) scale(${z})`;
      layer.style.setProperty("--z", String(z));
      layer.toggleAttribute("data-animate", animate);
      viewport.style.setProperty("--gx", `${x}px`);
      viewport.style.setProperty("--gy", `${y}px`);
      viewport.style.setProperty("--gs", `${GRID * z}px`);
      viewport.toggleAttribute("data-far", z < FAR_Z);
    };
    const prev = useFloorStore.getState();
    apply(prev);
    return useFloorStore.subscribe((s, p) => {
      if (s.camera !== p.camera || s.animate !== p.animate) apply(s);
    });
  }, []);

  // React's onWheel is passive, so preventDefault needs a native listener.
  useEffect(() => {
    const el = viewportRef.current!;
    const onWheel = (e: WheelEvent) => {
      if (!e.ctrlKey && !e.metaKey) return;
      e.preventDefault();
      const { camera, setCamera } = useFloorStore.getState();
      const box = el.getBoundingClientRect();
      setCamera(
        zoomAt(
          camera,
          e.clientX - box.left,
          e.clientY - box.top,
          Math.exp(-e.deltaY * 0.01),
        ),
      );
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, []);

  useEffect(() => {
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(([entry]) => {
      const { width: w, height: h } = entry.contentRect;
      const { viewport: was, focus, setViewport } = useFloorStore.getState();
      if (w === was.w && h === was.h) return;
      setViewport({ w, h });
      if (!w || !h) return;
      if (was.w && was.h && focus) jumpTo(focus);
    });
    observer.observe(viewportRef.current!);
    return () => observer.disconnect();
  }, []);

  // Fit all once per Floor visit (the flag survives a remount), as soon as the viewport and rooms are known, unless the camera moved first.
  // `moved`, not camera identity: StrictMode's simulated unmount runs Floor's reset, which replaces the camera.
  const hasViewport = useFloorStore(
    (s) => s.viewport.w > 0 && s.viewport.h > 0,
  );
  useEffect(() => {
    const { fitted, moved } = useFloorStore.getState();
    if (fitted || !hasViewport || rooms.length === 0) return;
    useFloorStore.setState({ fitted: true });
    if (!moved) jumpTo("all");
  }, [hasViewport, rooms.length]);

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0 || (e.target as Element).closest(NO_PAN)) return;
    pan.current = {
      id: e.pointerId,
      x: e.clientX,
      y: e.clientY,
      from: useFloorStore.getState().camera,
      moving: false,
    };
  };

  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    const p = pan.current;
    if (!p || p.id !== e.pointerId) return;
    // Released outside the viewport before capture: no pointerup reached us.
    if (e.buttons === 0) {
      pan.current = null;
      return;
    }
    const dx = e.clientX - p.x;
    const dy = e.clientY - p.y;
    if (!p.moving) {
      if (Math.hypot(dx, dy) < PAN_THRESHOLD) return;
      p.moving = true;
      // Capture only once panning, so a plain click still reaches its target.
      e.currentTarget.setPointerCapture?.(e.pointerId);
    }
    useFloorStore
      .getState()
      .setCamera({ x: p.from.x + dx, y: p.from.y + dy, z: p.from.z });
  };

  const onPointerEnd = (e: PointerEvent<HTMLDivElement>) => {
    const p = pan.current;
    if (!p || p.id !== e.pointerId) return;
    pan.current = null;
    e.currentTarget.releasePointerCapture?.(e.pointerId);
    if (!p.moving && e.type === "pointerup")
      useFloorStore.getState().closePopover();
  };

  const onKeyDown = (e: KeyboardEvent) => {
    const { popover, terminalId } = useFloorStore.getState();
    if (e.key === "Escape" && !popover && !terminalId) jumpTo("all");
  };

  return (
    <div
      ref={viewportRef}
      className={styles.viewport}
      role="region"
      aria-label="Floor map"
      tabIndex={-1}
      data-popover-keep
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerEnd}
      onPointerCancel={onPointerEnd}
      onKeyDown={onKeyDown}
    >
      <div ref={layerRef} className={styles.layer}>
        {rooms.map((room) => (
          <Room key={room.project.id} room={room} />
        ))}
      </div>
    </div>
  );
}
