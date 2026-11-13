import type {
  ProcessInfo,
  ProjectId,
  ProjectInfo,
  ScratchpadId,
  TodoId,
} from "../../ipc/types";

export type FloorMode = "project" | "floor";
export type DeskState = "working" | "needs" | "idle" | "ended" | "crashed";
export type FixtureKind = "todos" | "scratchpads" | "services" | "menu";
export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}
export interface Size {
  w: number;
  h: number;
}
export interface Camera {
  x: number;
  y: number;
  z: number;
}
export interface RoomModel {
  project: ProjectInfo;
  rect: Rect; // world px, snapped to 40
  agents: ProcessInfo[];
  terminals: ProcessInfo[]; // [] when sections.terminals is off
  services: ProcessInfo[];
  needs: number; // agents that wait on the user
  crashed: number; // crashed agents + crashed services
  lit: boolean; // an agent is working or waiting
}
export interface FloorPopover {
  projectId: ProjectId;
  kind: FixtureKind;
  itemId: string | null;
}
export interface AgentModalRequest {
  projectId: ProjectId;
  todoIds?: TodoId[];
  initialName?: string;
}
export interface ScratchpadAgentModalRequest {
  projectId: ProjectId;
  scratchpadIds: ScratchpadId[];
  initialName?: string;
}
