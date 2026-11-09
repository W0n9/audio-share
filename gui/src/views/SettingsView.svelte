<script lang="ts">
  import { server } from "../lib/store.svelte";
  import { t } from "../lib/i18n.svelte";
  import type { Encoding } from "../lib/ipc";

  // Server settings persist immediately on change. Application settings
  // (autostart, launch/close, update, language) are loaded from config.ini and
  // stay read-only until ticket 10 wires their behavior.
  const encodings: Encoding[] = ["default", "f32", "s8", "s16", "s24", "s32"];
  const locked = $derived(server.settingsLocked);

  function encodingLabel(value: Encoding): string {
    return value === "default" ? t("encoding.default") : value;
  }

  function persist(): void {
    void server.persist();
  }

  function reset(): void {
    void server.reset();
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
    <input type="checkbox" bind:checked={server.settings.autostart} disabled />
  </div>

  <div class="row">
    <div class="row-text">
      <div class="label">{t("settings.onLaunch")}</div>
      <div class="sub">{t("settings.onLaunchHint")}</div>
    </div>
    <select bind:value={server.settings.whenStart} disabled>
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
    <select bind:value={server.settings.whenClose} disabled>
      <option value="minimize">{t("settings.onCloseTray")}</option>
      <option value="exit">{t("settings.onCloseExit")}</option>
    </select>
  </div>

  <div class="row">
    <div class="row-text">
      <div class="label">{t("settings.autoUpdate")}</div>
      <div class="sub">{t("settings.autoUpdateHint")}</div>
    </div>
    <input type="checkbox" bind:checked={server.settings.autoUpdate} disabled />
  </div>

  <div class="row">
    <div class="row-text">
      <div class="label">{t("settings.language")}</div>
      <div class="sub">{t("settings.languageHint")}</div>
    </div>
    <select bind:value={server.settings.language} disabled>
      <option value="default">{t("settings.languageDefault")}</option>
      <option value="en-US">English</option>
      <option value="zh-CN">简体中文</option>
    </select>
  </div>

  {#if server.platform === "windows"}
    <div class="row">
      <div class="row-text">
        <div class="label">{t("settings.repairFirewall")}</div>
        <div class="sub">{t("settings.repairFirewallHint")}</div>
      </div>
      <button class="ghost" disabled>{t("action.repair")}</button>
    </div>
  {/if}

  <div class="row">
    <div class="row-text">
      <div class="label">{t("settings.about")}</div>
      <div class="sub">{t("settings.aboutHint", { version: server.version })}</div>
    </div>
    <button class="ghost" disabled>{t("action.open")}</button>
  </div>
</div>

<p class="placeholder">{t("settings.applicationPlaceholder")}</p>

<footer class="footer">
  <button class="link" onclick={() => (server.view = "main")}>{t("action.back")}</button>
  <button class="link muted" onclick={() => server.quit()}>{t("action.quit")}</button>
</footer>
