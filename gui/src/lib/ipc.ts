import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** Lifecycle state published by the Rust controller. */
export type ServerState = "stopped" | "starting" | "running" | "stopping";

/** Snapshot of the server controller. */
export interface ServerStatus {
  state: ServerState;
  address: string | null;
  lastError: string | null;
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
  running: boolean;
}

/** Application-level settings that may change while the server runs. */
export interface ApplicationSettings {
  autostart: boolean;
  whenStart: WhenAppStart;
  whenClose: WhenClose;
  autoUpdate: boolean;
  language: string;
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

/** One-time data fetched before the shell mounts. */
export interface Bootstrap {
  version: string;
  platform: string;
  /** Locale selected at startup (`en-US` or `zh-CN`). */
  locale: Locale;
  status: ServerStatus;
  settings: Settings;
  /** Structured migration/recovery warnings; empty when the config was clean. */
  configNotices: Notice[];
  /** An update already found by the startup check, if any. */
  update: UpdateCheckResult | null;
}

/**
 * Thin typed wrappers around the Tauri commands. Components and the store
 * never call `invoke` directly.
 */
export const api = {
  getBootstrap: () => invoke<Bootstrap>("get_bootstrap"),
  getStatus: () => invoke<ServerStatus>("get_status"),
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<Settings>("save_settings", { settings }),
  saveApplicationSettings: (settings: ApplicationSettings) =>
    invoke<Settings>("save_application_settings", { settings }),
  resetSettings: () => invoke<Settings>("reset_settings"),
  listEndpoints: () => invoke<Endpoint[]>("list_endpoints"),
  start: () => invoke<ServerStatus>("start_server"),
  stop: () => invoke<ServerStatus>("stop_server"),
  hideWindow: () => invoke<void>("hide_window"),
  exit: () => invoke<void>("exit_app"),
  checkUpdate: () => invoke<UpdateCheckResult>("check_update"),
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
