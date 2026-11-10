<script lang="ts">
  import { server } from "../lib/store.svelte";
  import { t } from "../lib/i18n.svelte";
  import { encodings, encodingLabel } from "../lib/encodings";
  import { statusLabel } from "../lib/status";

  const state = $derived(server.status.state);
  const running = $derived(state === "running");
  const transitioning = $derived(state === "starting" || state === "stopping");
  const locked = $derived(server.settingsLocked);
  const address = $derived(
    running && server.status.address
      ? server.status.address
      : `${server.settings.host}:${server.settings.port}`,
  );
  const error = $derived(server.error ?? server.status.lastError);
  const statusText = $derived(statusLabel(state));
  const actionText = $derived(
    state === "running"
      ? t("action.stop")
      : state === "starting"
        ? t("action.starting")
        : state === "stopping"
          ? t("action.stopping")
          : t("action.start"),
  );

  async function toggleServer(): Promise<void> {
    if (state === "running") {
      await server.stop();
    } else if (state === "stopped") {
      await server.start();
    }
  }

  function persist(): void {
    void server.persist();
  }
</script>

<section class="status-hero" aria-live="polite">
  <div class="status-line">
    <span class="dot" class:on={running} class:pending={transitioning}></span>{statusText}
  </div>
  <div class="address">{address}</div>
</section>

{#if error}
  <p class="error" role="alert">{error}</p>
{/if}
{#if server.saveError}
  <p class="error" role="alert">{server.saveError}</p>
{/if}

<div class="body">
  <button
    class="btn-primary"
    class:stop={running}
    disabled={server.busy || server.saving || transitioning}
    onclick={toggleServer}
  >
    {actionText}
  </button>

  <div class="field">
    <label for="endpoint">{t("field.endpoint")}</label>
    <select
      id="endpoint"
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
    <label for="encoding">{t("field.encoding")}</label>
    <select
      id="encoding"
      bind:value={server.settings.encoding}
      disabled={locked}
      onchange={persist}
    >
      {#each encodings as encoding (encoding)}
        <option value={encoding}>{encodingLabel(encoding)}</option>
      {/each}
    </select>
  </div>

  <div class="field-row">
    <div class="field">
      <label for="host">{t("field.host")}</label>
      <input
        id="host"
        type="text"
        bind:value={server.settings.host}
        disabled={locked}
        onchange={persist}
      />
    </div>
    <div class="field">
      <label for="port">{t("field.port")}</label>
      <input
        id="port"
        type="number"
        min="1"
        max="65535"
        bind:value={server.settings.port}
        disabled={locked}
        onchange={persist}
      />
    </div>
  </div>
</div>

<footer class="footer">
  <button class="link" onclick={() => (server.view = "settings")}>{t("action.settings")}</button>
  <button class="link muted" onclick={() => server.quit()}>{t("action.quit")}</button>
</footer>
