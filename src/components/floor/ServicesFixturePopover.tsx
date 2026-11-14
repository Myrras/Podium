import type { ProcessInfo, ProjectId } from "../../ipc/types";
import { openProcess } from "../../lib/floor/menus";
import { pillState, pillWord } from "../../lib/processStatus";
import { useProcessActivity } from "../../lib/useAgentActivity";
import { useFloorStore } from "../../state/floorStore";
import { useProcessStore } from "../../state/processStore";
import { useProjectStore } from "../../state/projectStore";
import { Popover } from "../Popover";
import styles from "./ServicesFixturePopover.module.css";

function ServiceRow({ service }: { service: ProcessInfo }) {
  const activity = useProcessActivity(service.id);
  const word = pillWord(
    pillState(service.status, service.kind.kind, activity),
    service.kind.kind,
  );
  return (
    <li>
      <button
        type="button"
        className={`ghost ${styles.row}`}
        onClick={() => openProcess(service.id)}
      >
        <span className={styles.name}>{service.name}</span>
        <span className={styles.status}>{word}</span>
      </button>
    </li>
  );
}

export function ServicesFixturePopover({
  projectId,
  anchor,
}: {
  projectId: ProjectId;
  itemId: string | null;
  anchor: HTMLElement | null;
}) {
  const closePopover = useFloorStore((s) => s.closePopover);
  const processes = useProcessStore((s) => s.processes);
  const project = useProjectStore((s) =>
    s.projects.find((p) => p.id === projectId),
  );
  const reload = useProjectStore((s) => s.reloadProjectConfig);
  const services = processes.filter(
    (p) => p.projectId === projectId && p.kind.kind === "service",
  );

  return (
    <Popover anchor={anchor} title="Services" onClose={closePopover}>
      {project?.configError && (
        <p className={styles.error} role="alert">
          {project.configError}
        </p>
      )}
      <ul className={styles.list}>
        {services.map((s) => (
          <ServiceRow key={s.id} service={s} />
        ))}
      </ul>
      <button type="button" onClick={() => void reload(projectId)}>
        Reload podium.yml
      </button>
    </Popover>
  );
}
