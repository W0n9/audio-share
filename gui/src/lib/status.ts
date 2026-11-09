import type { ServerState } from "./ipc";
import { t } from "./i18n.svelte";

/**
 * Localized label for a lifecycle state, shared by the shell header and the
 * main Server view.
 */
export function statusLabel(state: ServerState): string {
  switch (state) {
    case "running":
      return t("status.running");
    case "starting":
      return t("status.starting");
    case "stopping":
      return t("status.stopping");
    default:
      return t("status.stopped");
  }
}
