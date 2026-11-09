<script lang="ts">
  import { onMount } from "svelte";
  import type { UnlistenFn } from "@tauri-apps/api/event";
  import { server } from "./lib/store.svelte";
  import { onServerStateChanged, onSecondInstance, type ServerState } from "./lib/ipc";
  import { t } from "./lib/i18n.svelte";
  import ServerView from "./views/ServerView.svelte";
  import SettingsView from "./views/SettingsView.svelte";

  let unlistenState: UnlistenFn | undefined;
  let unlistenSecond: UnlistenFn | undefined;
  let disposed = false;

  onMount(() => {
    void server.init().catch((error) => (server.error = String(error)));

    // `listen()` resolves to an unlisten function; calling it before the
    // Promise settles would leak the listener, so handle the disposal race.
    void onServerStateChanged((status) => {
      server.status = status;
    }).then((unlisten) => {
      if (disposed) unlisten();
      else unlistenState = unlisten;
    });

    void onSecondInstance(() => {
      server.view = "main";
    }).then((unlisten) => {
      if (disposed) unlisten();
      else unlistenSecond = unlisten;
    });

    return () => {
      disposed = true;
      unlistenState?.();
      unlistenSecond?.();
    };
  });

  function statusLabel(state: ServerState): string {
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

  function toggleView(): void {
    server.view = server.view === "main" ? "settings" : "main";
  }
</script>

<div class="app">
  <header class="header">
    <div class="brand-icon" aria-hidden="true">♪</div>
    <div class="titles">
      <div class="name">{t("app.name")}</div>
      <div class="sub">
        {server.view === "settings" ? t("settings.title") : statusLabel(server.status.state)}
      </div>
    </div>
    <button
      class="icon-btn"
      aria-label={server.view === "main" ? t("action.settings") : t("action.back")}
      onclick={toggleView}
    >
      {server.view === "main" ? "⚙" : "←"}
    </button>
  </header>

  {#if server.view === "main"}
    <ServerView />
  {:else}
    <SettingsView />
  {/if}
</div>
