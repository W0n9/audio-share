import { api, type Bootstrap, type ServerStatus, type Settings } from "./ipc";

export type View = "main" | "settings";

const EMPTY_STATUS: ServerStatus = { state: "stopped", address: null, lastError: null };
const DEFAULT_SETTINGS: Settings = {
  host: "0.0.0.0",
  port: 65530,
  endpoint: "default",
  encoding: "default",
};

/** Reactive view state shared by the shell components. */
class ServerStore {
  status = $state<ServerStatus>({ ...EMPTY_STATUS });
  settings = $state<Settings>({ ...DEFAULT_SETTINGS });
  bootstrap = $state<Bootstrap | null>(null);
  view = $state<View>("main");
  busy = $state(false);
  error = $state<string | null>(null);

  get version(): string {
    return this.bootstrap?.version ?? "0.4.0";
  }

  get platform(): string {
    return this.bootstrap?.platform ?? "unknown";
  }

  /** Load version/platform/status/settings once the webview is ready. */
  async init(): Promise<void> {
    const data = await api.getBootstrap();
    this.bootstrap = data;
    this.status = data.status;
    this.settings = data.settings;
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
