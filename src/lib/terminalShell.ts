import { useSettingsStore } from "../state/settingsStore";

/**
 * The shell a new sidebar terminal launches, or `""` for the backend's
 * platform default ($SHELL -i / PowerShell). In WSL mode the setting names
 * the distro's shell, so a Windows-drive project running natively (opt-in)
 * gets the native default instead of e.g. `cmd /C zsh`.
 */
export function terminalShellFor(nativeInWslMode: boolean): string {
  return nativeInWslMode
    ? ""
    : useSettingsStore.getState().terminal.shell.trim();
}
