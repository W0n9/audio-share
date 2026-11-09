<script lang="ts">
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { server } from "../lib/store.svelte";
  import { t } from "../lib/i18n.svelte";
  import type { Encoding } from "../lib/ipc";
  import LinuxSettings from "./LinuxSettings.svelte";

  // Server settings persist immediately on change. Application settings
  // (autostart, launch/close, update, language) persist immediately too but
  // stay editable while the server runs; language applies on next launch.
  const encodings: Encoding[] = ["default", "f32", "s8", "s16", "s24", "s32"];
  const HOMEPAGE = "https://github.com/mkckr0/audio-share";
  const locked = $derived(server.settingsLocked);
  const appLocked = $derived(server.applicationLocked);
  let aboutOpen = $state(false);

  function encodingLabel(value: Encoding): string {
    return value === "default" ? t("encoding.default") : value;
  }

  function persist(): void {
    void server.persist();
  }

  function persistApplication(): void {
    void server.persistApplication();
  }

  function languageChanged(event: Event): void {
    void server.setLanguage((event.currentTarget as HTMLSelectElement).value);
  }

  function reset(): void {
    void server.reset();
  }

  async function openHomepage(): Promise<void> {
    try {
      await openUrl(HOMEPAGE);
    } catch (error) {
      server.error = String(error);
    }
  }
</script>

{#if server.saveError}
  <p class="error" role="alert">{server.saveError}</p>
{/if}

<div class="settings-list">
  <div class="section-title">{t("settings.serverSection")}</div>

  <div class="field-row server-fields">
    <div class="field">
      <label for="settings-host">{t("field.host")}</label>
      <input
        id="settings-host"
        type="text"
        bind:value={server.settings.host}
        disabled={locked}
        onchange={persist}
      />
    </div>
    <div class="field">
      <label for="settings-port">{t("field.port")}</label>
      <input
        id="settings-port"
        type="number"
        min="1"
        max="65535"
        bind:value={server.settings.port}
        disabled={locked}
        onchange={persist}
      />
    </div>
  </div>

  <div class="field">
    <label for="settings-endpoint">{t("field.endpoint")}</label>
    <select
      id="settings-endpoint"
      bind:value={server.settings.endpoint}
      disabled={locked}
      onchange={persist}
    >
      <option value="default">{t("endpoint.default")}</option>
      {#each server.endpointOptions as endpoint (endpoint.id)}
        <option value={endpoint.id}>{endpoint.name}</option>
      {/each}
    </select>
  </div>

  <div class="field">
    <label for="settings-encoding">{t("field.encoding")}</label>
    <select
      id="settings-encoding"
      bind:value={server.settings.encoding}
      disabled={locked}
      onchange={persist}
    >
      {#each encodings as encoding (encoding)}
        <option value={encoding}>{encodingLabel(encoding)}</option>
      {/each}
    </select>
  </div>

  <div class="row reset-row">
    <div class="row-text">
      <div class="label">{t("settings.reset")}</div>
      <div class="sub">{t("settings.resetHint")}</div>
    </div>
    <button class="ghost" disabled={locked} onclick={reset}>{t("action.reset")}</button>
  </div>

  <div class="section-title">{t("settings.applicationSection")}</div>

  <div class="row">
    <div class="row-text">
      <div class="label">{t("settings.startAtLogin")}</div>
      <div class="sub">{t("settings.startAtLoginHint")}</div>
    </div>
    <input
      type="checkbox"
      bind:checked={server.settings.autostart}
      disabled={appLocked}
      onchange={persistApplication}
    />
  </div>

  <div class="row">
    <div class="row-text">
      <div class="label">{t("settings.onLaunch")}</div>
      <div class="sub">{t("settings.onLaunchHint")}</div>
    </div>
    <select
      bind:value={server.settings.whenStart}
      disabled={appLocked}
      onchange={persistApplication}
    >
      <option value="nothing">{t("settings.onLaunchNothing")}</option>
      <option value="start">{t("settings.onLaunchStart")}</option>
      <option value="keepLast">{t("settings.onLaunchLast")}</option>
    </select>
  </div>

  <div class="row">
    <div class="row-text">
      <div class="label">{t("settings.onClose")}</div>
      <div class="sub">{t("settings.onCloseHint")}</div>
    </div>
    <select
      bind:value={server.settings.whenClose}
      disabled={appLocked}
      onchange={persistApplication}
    >
      <option value="minimize">{t("settings.onCloseTray")}</option>
      <option value="exit">{t("settings.onCloseExit")}</option>
    </select>
  </div>

  <div class="row">
    <div class="row-text">
      <div class="label">{t("settings.autoUpdate")}</div>
      <div class="sub">{t("settings.autoUpdateHint")}</div>
    </div>
    <input
      type="checkbox"
      bind:checked={server.settings.autoUpdate}
      disabled={appLocked}
      onchange={persistApplication}
    />
  </div>

  <div class="row">
    <div class="row-text">
      <div class="label">{t("settings.language")}</div>
      <div class="sub">{t("settings.languageHint")}</div>
      {#if server.languageRestartHint}
        <div class="sub hint">{t("settings.languageRestartHint")}</div>
      {/if}
    </div>
    <select
      value={server.settings.language}
      disabled={appLocked}
      onchange={languageChanged}
    >
      <option value="default">{t("settings.languageDefault")}</option>
      <option value="en-US">English</option>
      <option value="zh-CN">简体中文</option>
    </select>
  </div>

  <div class="row">
    <div class="row-text">
      <div class="label">{t("settings.checkUpdate")}</div>
      <div class="sub">{t("settings.checkUpdateHint")}</div>
    </div>
    <button
      class="ghost"
      disabled={server.updateBusy}
      onclick={() => server.checkUpdate()}
    >
      {t("action.checkUpdate")}
    </button>
  </div>

  {#if server.updateMessage}
    <p class="update-status" role="status">{server.updateMessage}</p>
  {/if}

  {#if server.platform === "windows"}
    <div class="row">
      <div class="row-text">
        <div class="label">{t("settings.repairFirewall")}</div>
        <div class="sub">{t("settings.repairFirewallHint")}</div>
      </div>
      <button class="ghost" disabled>{t("action.repair")}</button>
    </div>
  {/if}

  {#if server.platform === "linux"}
    <LinuxSettings />
  {/if}

  <div class="row">
    <div class="row-text">
      <div class="label">{t("settings.about")}</div>
      <div class="sub">{t("settings.aboutHint", { version: server.version })}</div>
    </div>
    <button class="ghost" onclick={() => (aboutOpen = true)}>{t("action.open")}</button>
  </div>
</div>

{#if aboutOpen}
  <div class="modal-backdrop">
    <div class="modal" role="dialog" aria-modal="true" aria-labelledby="about-title">
      <h2 id="about-title">{t("about.title")}</h2>
      <div class="about-product">{t("about.productName")}</div>
      <dl class="about-fields">
        <dt>{t("about.version")}</dt>
        <dd>{server.version}</dd>
        <dt>{t("about.homepage")}</dt>
        <dd>
          <button class="link" onclick={openHomepage}>{HOMEPAGE}</button>
        </dd>
        <dt>{t("about.license")}</dt>
        <dd>{t("about.licenseValue")}</dd>
      </dl>
      <p class="about-copyright">{t("about.copyright")}</p>
      <div class="about-actions">
        <button class="ghost" onclick={() => (aboutOpen = false)}>{t("about.close")}</button>
      </div>
    </div>
  </div>
{/if}

<footer class="footer">
  <button class="link" onclick={() => (server.view = "main")}>{t("action.back")}</button>
  <button class="link muted" onclick={() => server.hide()}>{t("action.hide")}</button>
  <button class="link muted" onclick={() => server.quit()}>{t("action.quit")}</button>
</footer>
