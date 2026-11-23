import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** Lifecycle state published by the Rust controller. */
export type ServerState = "stopped" | "starting" | "running" | "stopping";

/** Independent v2/QUIC lifecycle state. */
export type V2State = "disabled" | "starting" | "running" | "error";

/** Independent v2/QUIC status; a v2 error never changes the v1 state. */
export interface V2Status {
  state: V2State;
  address: string | null;
  fingerprint: string | null;
  lastError: string | null;
}

/** Snapshot of the server controller. */
export interface ServerStatus {
  state: ServerState;
  address: string | null;
  lastError: string | null;
  v2: V2Status;
}

/** Capture encoding names shared with `audio_share_core::config`. */
export type Encoding = "default" | "f32" | "s8" | "s16" | "s24" | "s32";

/** `[App Settings] WhenAppStart`. */
export type WhenAppStart = "nothing" | "start" | "keepLast";

/** `[App Settings] WhenClose`. */
export type WhenClose = "exit" | "minimize";

/** An interface language; Rust resolves `default` before the UI mounts. */
export type Locale = "en-US" | "zh-CN";

/**
 * GUI color scheme (`[App] theme`): `system` follows the operating system,
 * the other two pin the matching token set.
 */
export type Theme = "system" | "light" | "dark";

/**
 * Linux help topics replacing the Windows-only maintenance actions. Rust maps
 * each id to a fixed documentation URL; the webview never supplies a URL.
 */
export type HelpTopic = "pipewire" | "desktopSound";

/**
 * Persisted settings. The shape mirrors `audio_share_core::config::Config`
 * (serde camelCase); `running` is controller-owned.
 */
export interface Settings {
  host: string;
  port: number;
  endpoint: string;
  encoding: Encoding;
  autostart: boolean;
  whenStart: WhenAppStart;
  whenClose: WhenClose;
  autoUpdate: boolean;
  language: string;
  theme: Theme;
  running: boolean;
}

/** Application-level settings that may change while the server runs. */
export interface ApplicationSettings {
  autostart: boolean;
  whenStart: WhenAppStart;
  whenClose: WhenClose;
  autoUpdate: boolean;
  language: string;
  theme: Theme;
}

/** Structured migration/recovery/startup message; see `notice.<code>` keys. */
export interface Notice {
  code: string;
  params: Record<string, string>;
}

/** A GitHub release newer than the running version. */
export interface NewerUpdate {
  status: "newer";
  version: string;
  url: string;
}

/** Result of an update check. */
export type UpdateCheckResult = { status: "upToDate" } | NewerUpdate;

/** One capture endpoint from the live backend. */
export interface Endpoint {
  id: string;
  name: string;
}

/** One paired device in the device list. */
export interface ClientInfo {
  clientId: string;
  displayName: string;
  state: "active" | "revoked";
  createdAt: string;
  lastSeenAt: string;
}

/** Pairing mode: QR primary (SAS fallback) or SAS only. */
export type PairingMode = "qr" | "sas";

/** A pairing invitation shown in the dialog. */
export interface PairingInvitation {
  mode: PairingMode;
  qrUri: string | null;
  qrSvg: string | null;
  fingerprint: string;
  /** Lifecycle of the invitation; only `active` presents the QR. */
  state: "active" | "consumed" | "expired";
  /** Seconds until the invitation expires; refreshed by polling. */
  expiresInSecs: number;
}

/** One verified pairing awaiting accept/decline. */
export interface PairingRequestInfo {
  approvalId: number;
  clientId: string;
  displayName: string;
  mode: PairingMode;
  address: string;
  sas: string | null;
  /**
   * Saved display name when approving restores a revoked record in place
   * (ADR-0006); null for a new or active identity.
   */
  wouldRestore: string | null;
}

/** A live pairing shown before the Client confirms. */
export interface PairingLiveInfo {
  approvalId: number;
  clientId: string;
  displayName: string;
  mode: PairingMode;
  address: string;
  sas: string | null;
  fingerprint: string;
  /** Seconds until the pairing deadline; refreshed by polling. */
  expiresInSecs: number;
  /** Saved display name when approving restores a revoked record in place. */
  wouldRestore: string | null;
}

/** The polled pairing dialog state. */
export interface PairingStatus {
  active: boolean;
  invitation: PairingInvitation | null;
  live: PairingLiveInfo[];
  pending: PairingRequestInfo[];
  v2: V2Status;
}

/** One-time data fetched before the shell mounts. */
export interface Bootstrap {
  version: string;
  platform: string;
  /** Locale selected at startup (`en-US` or `zh-CN`). */
  locale: Locale;
  status: ServerStatus;
  settings: Settings;
  /** Local IPv4 addresses offered as Host suggestions, default first. */
  hostAddresses: string[];
  /** Structured migration/recovery warnings; empty when the config was clean. */
  configNotices: Notice[];
  /** An update already found by a background check, if any. */
  update: UpdateCheckResult | null;
}

/**
 * Thin typed wrappers around the Tauri commands. Components and the store
 * never call `invoke` directly.
 */
export const api = {
  getBootstrap: () => invoke<Bootstrap>("get_bootstrap"),
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<Settings>("save_settings", { settings }),
  saveApplicationSettings: (settings: ApplicationSettings) =>
    invoke<Settings>("save_application_settings", { settings }),
  resetSettings: () => invoke<Settings>("reset_settings"),
  listEndpoints: () => invoke<Endpoint[]>("list_endpoints"),
  start: () => invoke<ServerStatus>("start_server"),
  stop: () => invoke<ServerStatus>("stop_server"),
  pairingBegin: (mode: PairingMode) => invoke<PairingStatus>("pairing_begin", { mode }),
  pairingCancel: () => invoke<PairingStatus>("pairing_cancel"),
  /** Poll the open dialog; renews a naturally expired QR in place. */
  pairingPoll: () => invoke<PairingStatus>("pairing_poll"),
  pairingApprove: (approvalId: number) =>
    invoke<PairingStatus>("pairing_approve", { approvalId }),
  pairingDecline: (approvalId: number) =>
    invoke<PairingStatus>("pairing_decline", { approvalId }),
  clientsList: () => invoke<ClientInfo[]>("clients_list"),
  clientsRevoke: (clientId: string) =>
    invoke<ClientInfo>("clients_revoke", { clientId }),
  /** Permanently delete a revoked record (ADR-0006). */
  clientsDelete: (clientId: string) =>
    invoke<ClientInfo>("clients_delete", { clientId }),
  hideWindow: () => invoke<void>("hide_window"),
  exit: () => invoke<void>("exit_app"),
  checkUpdate: () => invoke<UpdateCheckResult>("check_update"),
  openHelp: (topic: HelpTopic) => invoke<void>("open_help", { topic }),
  /** Windows only: rebuild inbound TCP/UDP firewall rules (UAC prompt). */
  repairFirewall: () => invoke<void>("repair_firewall"),
  /** Windows only: open the Sound control panel without waiting. */
  openSoundPanel: () => invoke<void>("open_sound_panel"),
};

/** Subscribe to controller transitions. Returns an unlisten function. */
export function onServerStateChanged(
  handler: (status: ServerStatus) => void,
): Promise<UnlistenFn> {
  return listen<ServerStatus>("server:state-changed", (event) => handler(event.payload));
}

/** A second launch asks the first instance to surface its window. */
export function onSecondInstance(handler: () => void): Promise<UnlistenFn> {
  return listen("app:second-instance", () => handler());
}

/** A background check found a newer release. */
export function onUpdateAvailable(handler: (update: UpdateCheckResult) => void): Promise<UnlistenFn> {
  return listen<UpdateCheckResult>("update:available", (event) => handler(event.payload));
}
