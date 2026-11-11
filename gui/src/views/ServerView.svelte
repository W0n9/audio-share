<script lang="ts">
  import { server } from "../lib/store.svelte";
  import { t } from "../lib/i18n.svelte";
  import { encodings, encodingLabel } from "../lib/encodings";
  import { statusLabel } from "../lib/status";
  import Icon from "../lib/Icon.svelte";

  const state = $derived(server.status.state);
  const running = $derived(state === "running");
  const transitioning = $derived(state === "starting" || state === "stopping");
  /** Stop keeps its own visual while the stop request is in flight. */
  const showStop = $derived(running || state === "stopping");
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
  const footerHint = $derived(state === "stopped" ? t("main.hintReady") : t("main.hintLocked"));

  async function toggleServer(): Promise<void> {
    if (running) {
      await server.stop();
    } else if (state === "stopped") {
      await server.start();
    }
  }

  function persist(): void {
    void server.persist();
  }

  function reset(): void {
    void server.reset();
  }
</script>

<section class="view">
  <div class="view-scroll main-scroll">
    <section class="hero">
      <!-- The live region covers only the status content; the action button
           stays outside so assistive tech is not re-notified on its state. -->
      <div aria-live="polite">
        <span
          class="hero-dot"
          class:is-running={running}
          class:is-pending={transitioning}
          aria-hidden="true"
        ></span>
        <h1 class="hero-title">{statusText}</h1>
        <div class="address-chip">{address}</div>
      </div>
      <button
        class="main-action"
        class:stop={showStop}
        disabled={server.busy || server.saving || transitioning}
        onclick={toggleServer}
      >
        <span class="action-circle">
          <Icon name={showStop ? "stop" : "play"} size={25} />
        </span>
        <strong>{actionText}</strong>
      </button>
    </section>

    {#if error}
      <p class="error" role="alert">{error}</p>
    {/if}
    {#if server.saveError}
      <p class="error" role="alert">{server.saveError}</p>
    {/if}

    <section class="quick-settings">
      <div class="quick-row">
        <label for="endpoint"><Icon name="audio" size={16} />{t("field.endpoint")}</label>
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

      <div class="quick-row">
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
    </section>

    <details class="disclosure">
      <summary>
        <span><Icon name="globe" size={16} />{t("main.networkSection")}</span>
        <Icon name="chevron" size={15} />
      </summary>
      <div class="disclosure-content">
        <div class="network-row">
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
    </details>

    <div class="server-reset">
      <div class="server-reset-text">
        <div class="server-reset-label">{t("settings.reset")}</div>
        <div class="sub">{t("settings.resetHint")}</div>
      </div>
      <button class="link" disabled={locked} onclick={reset}>{t("action.reset")}</button>
    </div>
  </div>

  <footer class="footer">
    <span class="footer-hint">{footerHint}</span>
    <button class="link muted" onclick={() => server.quit()}>{t("action.quit")}</button>
  </footer>
</section>
