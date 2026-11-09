<script lang="ts">
  import { api, type HelpTopic } from "../lib/ipc";
  import { t } from "../lib/i18n.svelte";
  import { server } from "../lib/store.svelte";

  // Linux replacement for the Windows-only Repair Firewall and Sound Panel
  // actions. Every row opens a fixed documentation page through the Rust
  // opener command (`open_help`), so the webview never chooses a URL.
  async function open(topic: HelpTopic): Promise<void> {
    try {
      await api.openHelp(topic);
    } catch (error) {
      server.error = String(error);
    }
  }
</script>

<div class="section-title">{t("linux.sectionTitle")}</div>

<div class="row">
  <div class="row-text">
    <div class="label">{t("linux.windowsOnlyTitle")}</div>
    <div class="sub">{t("linux.windowsOnlyHint")}</div>
  </div>
</div>

<div class="row">
  <div class="row-text">
    <div class="label">{t("linux.audioSettings")}</div>
    <div class="sub">{t("linux.audioSettingsHint")}</div>
  </div>
  <button class="ghost" onclick={() => open("desktopSound")}>{t("action.open")}</button>
</div>

<div class="row">
  <div class="row-text">
    <div class="label">{t("linux.pipewireDocs")}</div>
    <div class="sub">{t("linux.pipewireDocsHint")}</div>
  </div>
  <button class="ghost" onclick={() => open("pipewire")}>{t("action.open")}</button>
</div>

<style>
  /* Chinese labels are short but the hint column can squeeze the button;
     keep the two-character action on one line. */
  .ghost {
    flex: none;
    white-space: nowrap;
  }
</style>
