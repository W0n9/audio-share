<script lang="ts">
  import { server } from "../lib/store.svelte";
  import { t, locales } from "../lib/i18n.svelte";

  // Ticket 08 renders the settings shell. Ticket 09 adds persistence and
  // ticket 10 adds autostart/lifecycle/update/About behaviour; these controls
  // stay disabled until then.
  let autostart = $state(false);
  let onLaunch = $state("nothing");
  let onClose = $state("tray");
  let autoUpdate = $state(true);
  let language = $state("en-US");
</script>

<div class="settings-list">
  <div class="row">
    <div class="row-text">
      <div class="label">{t("settings.startAtLogin")}</div>
      <div class="sub">{t("settings.startAtLoginHint")}</div>
    </div>
    <input type="checkbox" bind:checked={autostart} disabled />
  </div>

  <div class="row">
    <div class="row-text">
      <div class="label">{t("settings.onLaunch")}</div>
      <div class="sub">{t("settings.onLaunchHint")}</div>
    </div>
    <select bind:value={onLaunch} disabled>
      <option value="nothing">{t("settings.onLaunchNothing")}</option>
      <option value="start">{t("settings.onLaunchStart")}</option>
      <option value="last">{t("settings.onLaunchLast")}</option>
    </select>
  </div>

  <div class="row">
    <div class="row-text">
      <div class="label">{t("settings.onClose")}</div>
      <div class="sub">{t("settings.onCloseHint")}</div>
    </div>
    <select bind:value={onClose} disabled>
      <option value="tray">{t("settings.onCloseTray")}</option>
      <option value="exit">{t("settings.onCloseExit")}</option>
    </select>
  </div>

  <div class="row">
    <div class="row-text">
      <div class="label">{t("settings.autoUpdate")}</div>
      <div class="sub">{t("settings.autoUpdateHint")}</div>
    </div>
    <input type="checkbox" bind:checked={autoUpdate} disabled />
  </div>

  <div class="row">
    <div class="row-text">
      <div class="label">{t("settings.language")}</div>
      <div class="sub">{t("settings.languageHint")}</div>
    </div>
    <select bind:value={language} disabled>
      {#each locales as locale (locale.id)}
        <option value={locale.id}>{locale.label}</option>
      {/each}
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

<p class="placeholder">{t("settings.placeholder")}</p>

<footer class="footer">
  <button class="link" onclick={() => (server.view = "main")}>{t("action.back")}</button>
  <button class="link muted" onclick={() => server.quit()}>{t("action.quit")}</button>
</footer>
