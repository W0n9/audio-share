<script lang="ts">
  import { onMount } from "svelte";
  import type { UnlistenFn } from "@tauri-apps/api/event";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { server } from "./lib/store.svelte";
  import {
    onSecondInstance,
    onServerStateChanged,
    onUpdateAvailable,
    type Notice,
  } from "./lib/ipc";
  import { t } from "./lib/i18n.svelte";
  import Icon from "./lib/Icon.svelte";
  import ServerView from "./views/ServerView.svelte";
  import SettingsView from "./views/SettingsView.svelte";

  let unlistenState: UnlistenFn | undefined;
  let unlistenSecond: UnlistenFn | undefined;
  let unlistenUpdate: UnlistenFn | undefined;
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

    void onUpdateAvailable((update) => {
      server.applyUpdateResult(update);
    }).then((unlisten) => {
      if (disposed) unlisten();
      else unlistenUpdate = unlisten;
    });

    return () => {
      disposed = true;
      unlistenState?.();
      unlistenSecond?.();
      unlistenUpdate?.();
    };
  });

  function openSettings(): void {
    server.view = "settings";
  }

  function closeSettings(): void {
    server.view = "main";
  }

  function noticeText(notice: Notice): string {
    return t(`notice.${notice.code}`, notice.params);
  }

  async function openReleasePage(): Promise<void> {
    if (server.update?.status !== "newer") return;
    try {
      await openUrl(server.update.url);
    } catch (error) {
      server.error = String(error);
    }
  }
</script>

<div class="app">
  <!-- The header carries only the brand and the Settings entry; the server
       state lives once, in the main view's status area. -->
  <header class="header">
    {#if server.view === "main"}
      <div class="brand">
        <span class="brand-mark" aria-hidden="true"><Icon name="music" size={16} /></span>
        <span class="brand-name">{t("app.name")}</span>
      </div>
      <button class="icon-btn" aria-label={t("action.settings")} onclick={openSettings}>
        <Icon name="settings" size={18} />
      </button>
    {:else}
      <button class="icon-btn" aria-label={t("action.back")} onclick={closeSettings}>
        <Icon name="back" size={18} />
      </button>
      <strong class="header-title">{t("settings.title")}</strong>
      <span class="header-spacer" aria-hidden="true"></span>
    {/if}
  </header>

  {#if server.notices.length > 0}
    <div class="banner" role="status">
      <div class="banner-title">{t("config.warningTitle")}</div>
      {#each server.notices as notice, index (index)}
        <p class="banner-message">{noticeText(notice)}</p>
      {/each}
      <button class="link" onclick={() => server.dismissNotices()}>
        {t("action.dismiss")}
      </button>
    </div>
  {/if}

  {#if server.update?.status === "newer" && !server.updateDismissed}
    <div class="banner update-banner" role="status">
      <div class="banner-title">{t("update.bannerTitle")}</div>
      <p class="banner-message">
        {t("update.bannerMessage", { version: server.update.version })}
      </p>
      <button class="link" onclick={openReleasePage}>{t("update.openRelease")}</button>
      <button class="link muted" onclick={() => server.dismissUpdate()}>
        {t("action.dismiss")}
      </button>
    </div>
  {/if}

  {#if server.view === "main"}
    <ServerView />
  {:else}
    <SettingsView />
  {/if}
</div>
