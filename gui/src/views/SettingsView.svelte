<script lang="ts">
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { server } from "../lib/store.svelte";
  import { locales, t } from "../lib/i18n.svelte";
  import Icon from "../lib/Icon.svelte";
  import LinuxSettings from "./LinuxSettings.svelte";

  // Application settings persist immediately on change and stay editable
  // while the server runs; `applicationLocked` only covers in-flight saves.
  // Language applies on the next launch, the theme applies at once.
  const HOMEPAGE = "https://github.com/mkckr0/audio-share";
  const appLocked = $derived(server.applicationLocked);

  function persistApplication(): void {
    void server.persistApplication();
  }

  function languageChanged(event: Event): void {
    void server.setLanguage((event.currentTarget as HTMLSelectElement).value);
  }

  async function openHomepage(): Promise<void> {
    try {
      await openUrl(HOMEPAGE);
    } catch (error) {
      server.error = String(error);
    }
  }
</script>

<section class="view">
  <div class="view-scroll settings-scroll">
    {#if server.error}
      <p class="error" role="alert">{server.error}</p>
    {/if}
    {#if server.saveError}
      <p class="error" role="alert">{server.saveError}</p>
    {/if}

    <!-- Appearance stays expanded; the remaining groups are disclosures. -->
    <section class="appearance">
      <div class="section-title">{t("settings.appearanceSection")}</div>

      <div class="row">
        <div class="row-text">
          <div class="label">{t("settings.theme")}</div>
          <div class="sub">{t("settings.themeHint")}</div>
        </div>
        <div class="segmented" role="group" aria-label={t("settings.theme")}>
          <button
            type="button"
            class:active={server.settings.theme === "system"}
            aria-pressed={server.settings.theme === "system"}
            disabled={appLocked}
            onclick={() => void server.setTheme("system")}
          >
            {t("settings.themeSystem")}
          </button>
          <button
            type="button"
            class:active={server.settings.theme === "light"}
            aria-pressed={server.settings.theme === "light"}
            disabled={appLocked}
            onclick={() => void server.setTheme("light")}
          >
            {t("settings.themeLight")}
          </button>
          <button
            type="button"
            class:active={server.settings.theme === "dark"}
            aria-pressed={server.settings.theme === "dark"}
            disabled={appLocked}
            onclick={() => void server.setTheme("dark")}
          >
            {t("settings.themeDark")}
          </button>
        </div>
      </div>

      <div class="row">
        <div class="row-text">
          <label class="label" for="language">{t("settings.language")}</label>
          <div class="sub">{t("settings.languageHint")}</div>
          {#if server.languageRestartHint}
            <div class="sub hint">{t("settings.languageRestartHint")}</div>
          {/if}
        </div>
        <select
          id="language"
          value={server.settings.language}
          disabled={appLocked}
          onchange={languageChanged}
        >
          <option value="default">{t("settings.languageDefault")}</option>
          {#each locales as locale (locale.id)}
            <option value={locale.id}>{locale.label}</option>
          {/each}
        </select>
      </div>
    </section>

    <div class="settings-groups">
      <details class="disclosure" open>
        <summary>
          <span>{t("settings.startupSection")}</span>
          <Icon name="chevron" size={15} />
        </summary>
        <div class="disclosure-content">
          <div class="row">
            <div class="row-text">
              <label class="label" for="autostart">{t("settings.startAtLogin")}</label>
              <div class="sub">{t("settings.startAtLoginHint")}</div>
            </div>
            <input
              id="autostart"
              type="checkbox"
              bind:checked={server.settings.autostart}
              disabled={appLocked}
              onchange={persistApplication}
            />
          </div>

          <div class="row">
            <div class="row-text">
              <label class="label" for="when-launch">{t("settings.onLaunch")}</label>
              <div class="sub">{t("settings.onLaunchHint")}</div>
            </div>
            <select
              id="when-launch"
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
              <label class="label" for="when-close">{t("settings.onClose")}</label>
              <div class="sub">{t("settings.onCloseHint")}</div>
            </div>
            <select
              id="when-close"
              bind:value={server.settings.whenClose}
              disabled={appLocked}
              onchange={persistApplication}
            >
              <option value="minimize">{t("settings.onCloseTray")}</option>
              <option value="exit">{t("settings.onCloseExit")}</option>
            </select>
          </div>
        </div>
      </details>

      <details class="disclosure">
        <summary>
          <span>{t("settings.updatesSection")}</span>
          <Icon name="chevron" size={15} />
        </summary>
        <div class="disclosure-content">
          <div class="row">
            <div class="row-text">
              <label class="label" for="auto-update">{t("settings.autoUpdate")}</label>
              <div class="sub">{t("settings.autoUpdateHint")}</div>
            </div>
            <input
              id="auto-update"
              type="checkbox"
              bind:checked={server.settings.autoUpdate}
              disabled={appLocked}
              onchange={persistApplication}
            />
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
        </div>
      </details>

      {#if server.platform === "windows" || server.platform === "linux"}
        <details class="disclosure">
          <summary>
            <span>{t("settings.systemSection")}</span>
            <Icon name="chevron" size={15} />
          </summary>
          <div class="disclosure-content">
            {#if server.platform === "windows"}
              <div class="row">
                <div class="row-text">
                  <div class="label">{t("settings.repairFirewall")}</div>
                  <div class="sub">{t("settings.repairFirewallHint")}</div>
                </div>
                <button
                  class="ghost"
                  disabled={server.maintenanceBusy}
                  onclick={() => server.repairFirewall()}
                >
                  {t("action.repair")}
                </button>
              </div>

              <div class="row">
                <div class="row-text">
                  <div class="label">{t("settings.soundPanel")}</div>
                  <div class="sub">{t("settings.soundPanelHint")}</div>
                </div>
                <button
                  class="ghost"
                  disabled={server.maintenanceBusy}
                  onclick={() => server.openSoundPanel()}
                >
                  {t("action.openSoundPanel")}
                </button>
              </div>

              {#if server.maintenanceMessage}
                <p
                  class={server.maintenanceFailed ? "error" : "update-status"}
                  role={server.maintenanceFailed ? "alert" : "status"}
                >
                  {server.maintenanceMessage}
                </p>
              {/if}
            {:else if server.platform === "linux"}
              <LinuxSettings />
            {/if}
          </div>
        </details>
      {/if}

      <details class="disclosure">
        <summary>
          <span>{t("settings.about")}</span>
          <Icon name="chevron" size={15} />
        </summary>
        <div class="disclosure-content">
          <div class="about-block">
            <div class="about-name">{t("about.productName")}</div>
            <div class="about-meta">{t("settings.aboutHint", { version: server.version })}</div>
            <button class="link" onclick={openHomepage}>{t("about.homepage")}</button>
            <div class="about-meta">{t("about.copyright")}</div>
          </div>
        </div>
      </details>
    </div>
  </div>

  <footer class="footer">
    <button class="link" onclick={() => (server.view = "main")}>{t("action.back")}</button>
    <div class="footer-group">
      <button class="link muted" onclick={() => server.hide()}>{t("action.hide")}</button>
      <button class="link muted" onclick={() => server.quit()}>{t("action.quit")}</button>
    </div>
  </footer>
</section>
