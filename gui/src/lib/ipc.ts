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

/** In-memory settings shown by the shell (persistence arrives in ticket 09). */
export interface Settings {
  host: string;
  port: number;
  endpoint: string;
  encoding: string;
}

/** One-time data fetched when the window mounts. */
export interface Bootstrap {
  version: string;
  platform: string;
  status: ServerStatus;
  settings: Settings;
}

/**
 * Thin typed wrappers around the Tauri commands. Components and the store
 * never call `invoke` directly.
 */
export const api = {
  getBootstrap: () => invoke<Bootstrap>("get_bootstrap"),
  getStatus: () => invoke<ServerStatus>("get_status"),
  start: () => invoke<ServerStatus>("start_server"),
  stop: () => invoke<ServerStatus>("stop_server"),
  hideWindow: () => invoke<void>("hide_window"),
  exit: () => invoke<void>("exit_app"),
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
