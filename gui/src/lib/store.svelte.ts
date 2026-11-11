import {
  api,
  type ApplicationSettings,
  type Bootstrap,
  type Endpoint,
  type Notice,
  type ServerStatus,
  type Settings,
  type Theme,
  type UpdateCheckResult,
} from "./ipc";
import { applyTheme } from "./theme";
import { t } from "./i18n.svelte";

export type View = "main" | "settings";

const EMPTY_STATUS: ServerStatus = { state: "stopped", address: null, lastError: null };
// Pre-bootstrap fallback only; the Rust bootstrap always supplies the real
// settings. Keep in sync with `audio_share_core::config::DEFAULT_PORT`.
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
  theme: "system",
  running: false,
};

/** Reactive view state shared by the shell components. */
class ServerStore {
  status = $state<ServerStatus>({ ...EMPTY_STATUS });
  settings = $state<Settings>({ ...DEFAULT_SETTINGS });
  bootstrap = $state<Bootstrap | null>(null);
  endpoints = $state<Endpoint[]>([]);
  /** Structured migration/recovery warnings shown in the banner. */
  notices = $state<Notice[]>([]);
  /** Newest release found by any check; drives the in-app banner. */
  update = $state<UpdateCheckResult | null>(null);
  updateDismissed = $state(false);
  updateBusy = $state(false);
  /** Manual-check status text (checking / up to date / error). */
  updateMessage = $state<string | null>(null);
  /** Windows maintenance actions: firewall repair / Sound control panel. */
  maintenanceBusy = $state(false);
  maintenanceMessage = $state<string | null>(null);
  maintenanceFailed = $state(false);
  /** Persisted language changes need a restart, per spec. */
  languageRestartHint = $state(false);
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

  /** Application settings stay editable while the server runs (MFC parity). */
  get applicationLocked(): boolean {
    return this.busy || this.saving;
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

  /** Apply the pre-mount bootstrap payload (the locale is set in main.ts). */
  hydrate(data: Bootstrap): void {
    this.bootstrap = data;
    this.status = data.status;
    this.settings = data.settings;
    this.notices = data.configNotices;
    this.update = data.update;
    if (data.update?.status === "newer") {
      this.updateMessage = t("update.available", { version: data.update.version });
    }
    void this.refreshEndpoints();
  }

  /** Load once if `main.ts` did not manage to fetch the bootstrap. */
  async init(): Promise<void> {
    if (this.bootstrap) return;
    const data = await api.getBootstrap();
    this.hydrate(data);
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

  /**
   * Persist autostart/launch/close/update/language/theme settings. Allowed
   * while the server runs; Rust applies the login entry first for autostart.
   */
  async persistApplication(): Promise<void> {
    if (this.applicationLocked) return;
    const application: ApplicationSettings = {
      autostart: this.settings.autostart,
      whenStart: this.settings.whenStart,
      whenClose: this.settings.whenClose,
      autoUpdate: this.settings.autoUpdate,
      language: this.settings.language,
      theme: this.settings.theme,
    };
    this.saving = true;
    this.saveError = null;
    try {
      this.settings = await api.saveApplicationSettings(application);
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

  /** Change the language, persist it, and show the restart hint (no live switch). */
  async setLanguage(language: string): Promise<void> {
    this.settings.language = language;
    this.languageRestartHint = true;
    await this.persistApplication();
  }

  /**
   * Change the theme, apply it immediately, and persist it with the rest of
   * the application settings. If the save fails and the re-read restores the
   * stored value, the applied theme follows the file again.
   */
  async setTheme(theme: Theme): Promise<void> {
    if (this.settings.theme === theme) return;
    this.settings.theme = theme;
    applyTheme(theme);
    await this.persistApplication();
    if (this.settings.theme !== theme) applyTheme(this.settings.theme);
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

  /** A manual update check reports Up To Date, a new version, or an error. */
  async checkUpdate(): Promise<void> {
    if (this.updateBusy) return;
    this.updateBusy = true;
    this.updateMessage = t("update.checking");
    try {
      const result = await api.checkUpdate();
      this.applyUpdateResult(result);
    } catch (error) {
      this.updateMessage = describeUpdateError(error);
    } finally {
      this.updateBusy = false;
    }
  }

  /** Apply a manual or background result. */
  applyUpdateResult(result: UpdateCheckResult): void {
    this.update = result.status === "newer" ? result : null;
    if (result.status === "newer") {
      this.updateDismissed = false;
      this.updateMessage = t("update.available", { version: result.version });
    } else {
      this.updateMessage = t("update.upToDate");
    }
  }

  dismissUpdate(): void {
    this.updateDismissed = true;
  }

  /**
   * Windows firewall repair. Runs an elevated netsh script in Rust; this
   * method only tracks the localized success/error text.
   */
  async repairFirewall(): Promise<void> {
    if (this.maintenanceBusy) return;
    this.maintenanceBusy = true;
    this.maintenanceFailed = false;
    this.maintenanceMessage = null;
    try {
      await api.repairFirewall();
      this.maintenanceMessage = t("maintenance.firewallSuccess");
    } catch (error) {
      this.maintenanceMessage = String(error);
      this.maintenanceFailed = true;
    } finally {
      this.maintenanceBusy = false;
    }
  }

  /** Windows Sound control panel; Rust spawns it without waiting. */
  async openSoundPanel(): Promise<void> {
    if (this.maintenanceBusy) return;
    this.maintenanceBusy = true;
    this.maintenanceFailed = false;
    this.maintenanceMessage = null;
    try {
      await api.openSoundPanel();
      this.maintenanceMessage = t("maintenance.soundPanelOpened");
    } catch (error) {
      this.maintenanceMessage = String(error);
      this.maintenanceFailed = true;
    } finally {
      this.maintenanceBusy = false;
    }
  }

  async hide(): Promise<void> {
    try {
      await api.hideWindow();
    } catch (error) {
      this.error = String(error);
    }
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

/** Avoid `Update check failed: update check failed: ...` duplication. */
function describeUpdateError(error: unknown): string {
  let message = String(error);
  const prefix = "update check failed: ";
  if (message.toLowerCase().startsWith(prefix)) message = message.slice(prefix.length);
  return t("update.error", { message });
}

export const server = new ServerStore();
