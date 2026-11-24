import {
  api,
  type ApplicationSettings,
  type Bootstrap,
  type ClientInfo,
  type Endpoint,
  type Notice,
  type PairingMode,
  type PairingStatus,
  type ServerStatus,
  type Settings,
  type Theme,
  type UpdateCheckResult,
} from "./ipc";
import { applyTheme } from "./theme";
import { t } from "./i18n.svelte";

export type View = "main" | "settings";

const EMPTY_STATUS: ServerStatus = {
  state: "stopped",
  address: null,
  lastError: null,
  v2: { state: "disabled", address: null, fingerprint: null, lastError: null },
};
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

/**
 * One pairing's success lifecycle. Recorded at approval (`pending` until the
 * device list shows the record active), then the single source for the success
 * line, the consumed-QR suppression, and the request identity a stale poll
 * must not treat as a new attempt.
 */
interface PairingSuccess {
  clientId: string;
  approvalId: number;
  name: string;
  /** The device list has not yet shown the approved record active. */
  pending: boolean;
  /** Polls spent waiting (bounded, so a failed persistence stops waiting). */
  ticks: number;
  /** The success line was dismissed by reopening the dialog. */
  dismissed: boolean;
}

/** Reactive view state shared by the shell components. */
class ServerStore {
  status = $state<ServerStatus>({ ...EMPTY_STATUS });
  settings = $state<Settings>({ ...DEFAULT_SETTINGS });
  bootstrap = $state<Bootstrap | null>(null);
  endpoints = $state<Endpoint[]>([]);
  /** Local addresses offered as Host suggestions: IPv4 first, then IPv6. */
  hostAddresses = $state<string[]>([]);
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
  /** Pairing dialog visibility and polled pairing/device state. */
  pairingOpen = $state(false);
  pairing = $state<PairingStatus | null>(null);
  clients = $state<ClientInfo[]>([]);
  pairingBusy = $state(false);
  pairingError = $state<string | null>(null);
  revokingClientId = $state<string | null>(null);
  deletingClientId = $state<string | null>(null);
  /**
   * Revoked device the current invitation was opened for. Matching live and
   * pending connections are labelled with its saved name (spec: Re-pairing).
   */
  repairTarget = $state<{ clientId: string; displayName: string } | null>(null);
  /**
   * The one success record for the current invitation: `pending` while the
   * approved device awaits persistence, then the visible success line and the
   * request identity an in-flight poll must still recognise.
   */
  private success = $state<PairingSuccess | null>(null);
  private pairingTimer: ReturnType<typeof setInterval> | null = null;
  /** The current `pairingError` came from polling, so a good poll may clear it. */
  private pairingErrorFromPoll = false;

  /** One-line confirmation shown once an approved pairing is persisted. */
  get pairedName(): string | null {
    const success = this.success;
    return success && !success.pending && !success.dismissed
      ? success.name
      : null;
  }

  /**
   * The current invitation has already produced a persisted pairing. A
   * consumed QR is then expected, not a failure, even after the success line
   * is dismissed by reopening the dialog.
   */
  get pairingSucceeded(): boolean {
    return this.success !== null && !this.success.pending;
  }

  /** True while an approval is waiting for the device list to reflect it. */
  get pairingInFlight(): boolean {
    return this.success?.pending === true;
  }

  get version(): string {
    return this.bootstrap?.version ?? "unknown";
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
    this.hostAddresses = data.hostAddresses;
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
   * reported without a command round-trip; in both the local and Rust-side
   * failure paths the last saved settings are restored so the UI never lies
   * about the file.
   */
  async persist(): Promise<void> {
    if (this.settingsLocked) return;
    const { host, error } = normalizeHost(this.settings.host);
    const port = Number(this.settings.port);
    if (error) {
      this.saveError = t(error);
      await this.restoreSavedSettings();
      return;
    }
    if (!Number.isInteger(port) || port < 1 || port > 65535) {
      this.saveError = t("settings.errorPort");
      await this.restoreSavedSettings();
      return;
    }

    this.saving = true;
    this.saveError = null;
    try {
      this.settings = await api.saveSettings({ ...this.settings, host, port });
      this.saveError = null;
    } catch (error) {
      this.saveError = String(error);
      await this.restoreSavedSettings();
    } finally {
      this.saving = false;
    }
  }

  /** Restore the backend's saved settings after a refused or failed save. */
  private async restoreSavedSettings(): Promise<void> {
    try {
      this.settings = await api.getSettings();
    } catch {
      // Keep the user's input visible; the message already explains failure.
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
   * stored value, the applied theme follows the file again. While a save is
   * in flight (`applicationLocked`) the change is dropped instead: applying
   * optimistically there would leave the document themed with neither a save
   * nor a corrective re-read behind it.
   */
  async setTheme(theme: Theme): Promise<void> {
    if (this.settings.theme === theme) return;
    if (this.applicationLocked) return;
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

  /** Switch to the settings view. */
  openSettings(): void {
    this.view = "settings";
  }

  /** Return from the settings view to the main view. */
  closeSettings(): void {
    this.view = "main";
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

  /** Apply a status snapshot; the pairing dialog closes when the server stops. */
  applyStatus(status: ServerStatus): void {
    this.status = status;
    if (status.state !== "running") {
      this.stopPairingPoll();
      this.pairing = null;
      this.clients = [];
      this.repairTarget = null;
      this.success = null;
      this.pairingErrorFromPoll = false;
    }
  }

  /** Open the pairing dialog and start polling (no server command yet). */
  openPairing(): void {
    this.pairingOpen = true;
    this.pairingError = null;
    // Reopening dismisses the visible success line but keeps the invitation's
    // succeeded state, so a consumed QR is still expected rather than failed.
    if (this.success) this.success.dismissed = true;
    void this.refreshPairing();
    void this.refreshClients();
    this.startPairingPoll();
  }

  /**
   * Close the dialog. Closing does not consume QR nonces; they expire on their
   * own (pairing.md §5). The explicit Cancel action uses `cancelPairing`.
   */
  closePairing(): void {
    this.pairingOpen = false;
    this.stopPairingPoll();
  }

  private startPairingPoll(): void {
    this.stopPairingPoll();
    // Poll rather than bridge a broadcast channel: the command surface stays
    // the single seam, and pending requests appear within one second.
    this.pairingTimer = setInterval(() => this.poll(), 1000);
  }

  private poll(): void {
    void this.refreshPairing();
    if (this.success?.pending) void this.confirmPaired();
  }

  private stopPairingPoll(): void {
    if (this.pairingTimer !== null) {
      clearInterval(this.pairingTimer);
      this.pairingTimer = null;
    }
  }

  async refreshPairing(): Promise<void> {
    try {
      this.pairing = await api.pairingPoll();
      // A poll error is transient; an action error stays until the next action.
      if (this.pairingErrorFromPoll) {
        this.pairingError = null;
        this.pairingErrorFromPoll = false;
      }
      // A new attempt invalidates the previous success context: do not let an
      // old success line (or a consumed-QR suppression) outlive its invitation.
      // A poll already in flight during approval still carries the approved
      // request itself, so match on identity rather than on list emptiness.
      if (
        this.pairing.live.some((request) => !this.isSucceededRequest(request)) ||
        this.pairing.pending.some((request) => !this.isSucceededRequest(request))
      ) {
        this.success = null;
      }
    } catch (error) {
      this.pairingError = String(error);
      this.pairingErrorFromPoll = true;
    }
  }

  /**
   * Open a fresh invitation. `keepRepairTarget` keeps the revoked-device label
   * when refreshing an in-progress Re-pair invitation.
   */
  async beginPairing(mode: PairingMode, keepRepairTarget = false): Promise<void> {
    if (this.pairingBusy) return;
    this.pairingBusy = true;
    this.pairingError = null;
    try {
      this.pairing = await api.pairingBegin(mode);
      if (!keepRepairTarget) this.repairTarget = null;
      this.success = null;
    } catch (error) {
      this.failAction(error);
    } finally {
      this.pairingBusy = false;
    }
  }

  /** Start a fresh invitation for a revoked device (ADR-0006 Re-pairing). */
  async repairClient(clientId: string, displayName: string): Promise<void> {
    if (this.pairingBusy) return;
    this.pairingBusy = true;
    this.pairingError = null;
    try {
      this.pairing = await api.pairingBegin("qr");
      this.repairTarget = { clientId, displayName };
      this.success = null;
    } catch (error) {
      this.failAction(error);
    } finally {
      this.pairingBusy = false;
    }
  }

  async cancelPairing(): Promise<void> {
    if (this.pairingBusy) return;
    this.pairingBusy = true;
    this.pairingError = null;
    try {
      this.pairing = await api.pairingCancel();
      this.repairTarget = null;
      this.success = null;
    } catch (error) {
      this.failAction(error);
    } finally {
      this.pairingBusy = false;
    }
  }

  /** Approve a pending pairing with a plain Accept (never restores). */
  async approvePairing(approvalId: number): Promise<void> {
    await this.decidePairing(approvalId, api.pairingApprove);
  }

  /** Approve a revoked identity with the explicit Restore decision (ADR-0006). */
  async restorePairing(approvalId: number): Promise<void> {
    await this.decidePairing(approvalId, api.pairingRestore);
  }

  private async decidePairing(
    approvalId: number,
    decide: (id: number) => Promise<PairingStatus>,
  ): Promise<void> {
    if (this.pairingBusy) return;
    const request = this.pairing?.pending.find(
      (item) => item.approvalId === approvalId,
    );
    this.pairingBusy = true;
    this.pairingError = null;
    this.success = null;
    try {
      this.pairing = await decide(approvalId);
      // The driver persists after `decide` returns, so the device list may
      // still be stale here. Wait for it in `confirmPaired`.
      if (request) {
        this.success = {
          clientId: request.clientId,
          approvalId: request.approvalId,
          name: request.restoreCandidate ?? request.displayName,
          pending: true,
          ticks: 0,
          dismissed: false,
        };
      }
      await this.confirmPaired();
    } catch (error) {
      this.failAction(error);
    } finally {
      this.pairingBusy = false;
    }
  }

  /** Wait for an approved device to appear active, then show the success line. */
  private async confirmPaired(): Promise<void> {
    const pending = this.success;
    if (!pending?.pending) return;
    await this.refreshClients();
    // A begin, cancel, stop, or newer approval may have replaced the attempt
    // while the list was in flight; never resurrect stale success state.
    if (this.success !== pending) return;
    const client = this.clients.find((item) => item.clientId === pending.clientId);
    if (client?.state === "active") {
      this.success = { ...pending, pending: false };
      if (this.repairTarget?.clientId === pending.clientId) {
        this.repairTarget = null;
      }
      return;
    }
    pending.ticks += 1;
    // Persistence failures fail closed and never reach the device list; stop
    // waiting instead of showing success for a pairing the store rejected.
    if (pending.ticks > 15) this.success = null;
  }

  /** Whether `request` is the approved request the success line belongs to. */
  private isSucceededRequest(request: {
    clientId: string;
    approvalId: number;
  }): boolean {
    return (
      this.success?.clientId === request.clientId &&
      this.success?.approvalId === request.approvalId
    );
  }

  /** Record a command failure; only the next action (not a poll) clears it. */
  private failAction(error: unknown): void {
    this.pairingError = String(error);
    this.pairingErrorFromPoll = false;
  }

  async declinePairing(approvalId: number): Promise<void> {
    if (this.pairingBusy) return;
    this.pairingBusy = true;
    this.pairingError = null;
    try {
      this.pairing = await api.pairingDecline(approvalId);
      this.success = null;
    } catch (error) {
      this.failAction(error);
    } finally {
      this.pairingBusy = false;
    }
  }

  async refreshClients(): Promise<void> {
    try {
      this.clients = await api.clientsList();
    } catch (error) {
      this.pairingError = String(error);
    }
  }

  /** Revoke a device; the server closes its active v2 session within 5 s. */
  async revokeClient(clientId: string): Promise<void> {
    if (this.revokingClientId !== null) return;
    this.revokingClientId = clientId;
    this.pairingError = null;
    try {
      const revoked = await api.clientsRevoke(clientId);
      this.clients = this.clients.map((client) =>
        client.clientId === revoked.clientId ? revoked : client,
      );
    } catch (error) {
      this.failAction(error);
    } finally {
      this.revokingClientId = null;
    }
  }

  /**
   * Permanently delete a revoked device record (ADR-0006). An active record is
   * refused by the core, so the device row offers this only after Revoke.
   */
  async deleteClient(clientId: string): Promise<void> {
    if (this.deletingClientId !== null) return;
    this.deletingClientId = clientId;
    this.pairingError = null;
    try {
      const deleted = await api.clientsDelete(clientId);
      this.clients = this.clients.filter(
        (client) => client.clientId !== deleted.clientId,
      );
      if (this.repairTarget?.clientId === deleted.clientId) {
        this.repairTarget = null;
      }
    } catch (error) {
      this.failAction(error);
    } finally {
      this.deletingClientId = null;
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
      this.applyStatus(await command());
    } catch (error) {
      // The Rust side already published a Stopped+lastError snapshot; keep
      // its readable message in the view as well.
      this.error = String(error);
    } finally {
      this.busy = false;
    }
  }
}

/**
 * Trim a GUI Host, strip paired brackets around an IPv6 literal, and return
 * the first localization key when it cannot be saved. Rust enforces the same
 * rules authoritatively in `save_settings_with_endpoints`; these checks exist
 * so the message is localized and the rollback happens without a round-trip.
 */
function normalizeHost(raw: string): { host: string; error: string | null } {
  let host = raw.trim();
  if (host.startsWith("[") && host.endsWith("]")) {
    const inner = host.slice(1, -1);
    if (inner.includes(":")) host = inner;
  }
  if (!host) return { host, error: "settings.errorHost" };
  if (host.includes("%")) return { host, error: "settings.errorHostScope" };
  if (!isValidHost(host)) return { host, error: "settings.errorHostInvalid" };
  return { host, error: null };
}

/** IPv4/hostname alphabet, or a syntactically valid IPv6 literal. */
function isValidHost(host: string): boolean {
  if (host.includes(":")) {
    if (host.includes("[") || host.includes("]")) return false;
    try {
      // The URL parser validates IPv6 grammar, so a malformed literal gets
      // the localized error instead of a raw Rust save error.
      new URL(`http://[${host}]/`);
      return true;
    } catch {
      return false;
    }
  }
  if (host.length > 253) return false;
  return host.split(".").every(
    (label) =>
      label.length > 0 &&
      label.length <= 63 &&
      /^[A-Za-z0-9][A-Za-z0-9-]*$/.test(label) &&
      /[A-Za-z0-9]$/.test(label),
  );
}

/** Avoid `Update check failed: update check failed: ...` duplication. */
function describeUpdateError(error: unknown): string {
  let message = String(error);
  const prefix = "update check failed: ";
  if (message.toLowerCase().startsWith(prefix)) message = message.slice(prefix.length);
  return t("update.error", { message });
}

export const server = new ServerStore();
