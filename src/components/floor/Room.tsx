import { memo, type CSSProperties } from "react";
import { DESK_W, WALL_H } from "../../lib/floor/geometry";
import type { RoomModel } from "../../lib/floor/types";
import { Desk } from "./Desk";
import styles from "./Room.module.css";
import { RoomNameplate } from "./RoomNameplate";
import { RoomWall } from "./RoomWall";

export const Room = memo(function Room({ room }: { room: RoomModel }) {
  const { rect, project } = room;
  const style = {
    left: rect.x,
    top: rect.y,
    width: rect.w,
    height: rect.h,
    "--pc": project.color ?? undefined,
    "--wall-h": `${WALL_H}px`,
    "--desk-w": `${DESK_W}px`,
  } as CSSProperties;
  return (
    <section
      className={styles.room}
      style={style}
      data-room={project.id}
      data-lit={room.lit}
      data-tinted={project.color ? "true" : "false"}
    >
      <div className={styles.plate}>
        <RoomNameplate room={room} />
      </div>
      <div className={styles.wall}>
        <RoomWall room={room} />
      </div>
      <div className={styles.desks}>
        {room.agents.map((p) => (
          <Desk key={p.id} process={p} />
        ))}
        {room.terminals.map((p) => (
          <Desk key={p.id} process={p} />
        ))}
      </div>
    </section>
  );
});
