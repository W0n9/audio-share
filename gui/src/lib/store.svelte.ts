import { api, type Bootstrap, type Endpoint, type ServerStatus, type Settings } from "./ipc";
import { t } from "./i18n.svelte";

export type View = "main" | "settings";

const EMPTY_STATUS: ServerStatus = { state: "stopped", address: null, lastError: null };
const DEFAULT_SETTINGS: Settings = {
  host: "0.0.0.0",
  port: 65530,
  endpoint: "default",
  encoding: "default",
  autostart: false,
  whenStart: "nothing",
  whenClose: "minimize",
  autoUpdate: false,
  language: "default",
  running: false,
};

/** Reactive view state shared by the shell components. */
class ServerStore {
  status = $state<ServerStatus>({ ...EMPTY_STATUS });
  settings = $state<Settings>({ ...DEFAULT_SETTINGS });
  bootstrap = $state<Bootstrap | null>(null);
  endpoints = $state<Endpoint[]>([]);
  /** Migration/recovery warnings shown once in the banner. */
  notices = $state<string[]>([]);
  view = $state<View>("main");
  busy = $state(false);
  saving = $state(false);
  error = $state<string | null>(null);
  saveError = $state<string | null>(null);

  get version(): string {
    return this.bootstrap?.version ?? "0.4.0";
  }

  get platform(): string {
    return this.bootstrap?.platform ?? "unknown";
  }

  /** Server fields are editable only while stopped. */
  get settingsLocked(): boolean {
    return this.busy || this.saving || this.status.state !== "stopped";
  }

  /** Default followed by live endpoints, plus any configured ID not listed. */
  get endpointOptions(): Endpoint[] {
    const options = this.endpoints.filter((endpoint) => endpoint.id !== "default");
    if (
      this.settings.endpoint !== "default" &&
      !options.some((endpoint) => endpoint.id === this.settings.endpoint)
    ) {
      options.unshift({ id: this.settings.endpoint, name: this.settings.endpoint });
    }
    return options;
  }

  /** Load version/platform/status/settings once the webview is ready. */
  async init(): Promise<void> {
    const data = await api.getBootstrap();
    this.bootstrap = data;
    this.status = data.status;
    this.settings = data.settings;
    this.notices = data.configNotices;
    void this.refreshEndpoints();
  }

  /** Re-enumerate real endpoints; keep the previous list on failure. */
  async refreshEndpoints(): Promise<void> {
    try {
      this.endpoints = await api.listEndpoints();
    } catch (error) {
      // Endpoint enumeration may be unavailable (no audio daemon). The
      // "Default" choice remains usable, and load-time validation was skipped.
      console.warn("Audio endpoint enumeration failed:", error);
    }
  }

  /**
   * Persist the current Server fields immediately. Invalid local input is
   * reported without a command round-trip; a Rust-side validation error
   * restores the last saved values so the UI never lies about the file.
   */
  async persist(): Promise<void> {
    if (this.settingsLocked) return;
    const host = this.settings.host.trim();
    const port = Number(this.settings.port);
    if (!host) {
      this.saveError = t("settings.errorHost");
      return;
    }
    if (!Number.isInteger(port) || port < 1 || port > 65535) {
      this.saveError = t("settings.errorPort");
      return;
    }

    this.saving = true;
    this.saveError = null;
    try {
      this.settings = await api.saveSettings({ ...this.settings, host, port });
    } catch (error) {
      this.saveError = String(error);
      try {
        this.settings = await api.getSettings();
      } catch {
        // Keep the user's input visible; the message already explains failure.
      }
    } finally {
      this.saving = false;
    }
  }

  /** Restore Host/Port/Endpoint/Encoding defaults and write them. */
  async reset(): Promise<void> {
    if (this.settingsLocked) return;
    this.saving = true;
    this.saveError = null;
    try {
      this.settings = await api.resetSettings();
    } catch (error) {
      this.saveError = String(error);
    } finally {
      this.saving = false;
    }
  }

  dismissNotices(): void {
    this.notices = [];
  }

  async start(): Promise<void> {
    await this.run(api.start);
  }

  async stop(): Promise<void> {
    await this.run(api.stop);
  }

  async quit(): Promise<void> {
    try {
      await api.exit();
    } catch (error) {
      this.error = String(error);
    }
  }

  private async run(command: () => Promise<ServerStatus>): Promise<void> {
    this.busy = true;
    this.error = null;
    try {
      this.status = await command();
    } catch (error) {
      // The Rust side already published a Stopped+lastError snapshot; keep
      // its readable message in the view as well.
      this.error = String(error);
    } finally {
      this.busy = false;
    }
  }
}

export const server = new ServerStore();
