/** Default main-area content while no process is focused. */

import { useProjectStore } from "../state/projectStore";
import { LogoMark } from "./LogoMark";
import { CloseIcon, FolderOpenIcon } from "./icons";
import styles from "./WelcomeScreen.module.css";

/** Keep the welcome card scannable; the backend remembers up to 20. */
const MAX_RECENTS_SHOWN = 8;

export function WelcomeScreen() {
  const hasProject = useProjectStore((s) => s.projects.length > 0);
  const openProjectDialog = useProjectStore((s) => s.openProjectDialog);
  const recents = useProjectStore((s) => s.recents);
  const openProject = useProjectStore((s) => s.openProject);
  const removeRecent = useProjectStore((s) => s.removeRecent);

  return (
    <div className={styles.welcome}>
      <LogoMark size={48} className={styles.logo} aria-hidden />
      <h1 className={styles.title}>Welcome to Podium</h1>
      <p className={styles.subtitle}>
        {hasProject
          ? "Select a terminal or process in the sidebar, or create one."
          : "Add a project to the sidebar to start terminals, processes, and agents."}
      </p>
      {!hasProject && (
        <button
          type="button"
          className={styles.openBtn}
          onClick={() => void openProjectDialog()}
        >
          <FolderOpenIcon size={15} />
          Add Project…
        </button>
      )}
      {!hasProject && recents.length > 0 && (
        <div className={styles.recents}>
          <h2 className={styles.recentsTitle}>Recent projects</h2>
          <ul className={styles.recentsList}>
            {recents.slice(0, MAX_RECENTS_SHOWN).map((r) => (
              <li key={r.path} className={styles.recentRow}>
                <button
                  type="button"
                  className={styles.recentOpen}
                  title={r.path}
                  onClick={() => void openProject(r.path)}
                >
                  <FolderOpenIcon size={13} className={styles.recentIcon} />
                  <span className={styles.recentName}>{r.name}</span>
                  <span className={styles.recentPath}>{r.path}</span>
                </button>
                <button
                  type="button"
                  className={styles.recentRemove}
                  aria-label={`Remove ${r.name} from recent projects`}
                  title="Remove from recents"
                  onClick={() => void removeRecent(r.path)}
                >
                  <CloseIcon size={12} />
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}
