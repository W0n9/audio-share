<script lang="ts">
  import { server } from "../lib/store.svelte";
  import { t } from "../lib/i18n.svelte";
  import { encodings, encodingLabel } from "../lib/encodings";
  import { statusLabel } from "../lib/status";
  import ErrorLine from "../lib/ErrorLine.svelte";
  import Icon from "../lib/Icon.svelte";

  const serverState = $derived(server.status.state);
  const running = $derived(serverState === "running");
  const transitioning = $derived(
    serverState === "starting" || serverState === "stopping",
  );
  /** Stop keeps its own visual while the stop request is in flight. */
  const showStop = $derived(running || serverState === "stopping");
  const locked = $derived(server.settingsLocked);
  /**
   * Sentinel for the explicit custom-entry option. The main control lists
   * every enumerated address; it never accepts free text itself.
   */
  const HOST_CUSTOM = "__custom__";
  /**
   * Custom entry is active either because the operator chose it or because
   * the saved Host is not one of the enumerated addresses (for example a
   * hostname or an address from a hand-edited config).
   */
  let customMode = $state(false);
  const hostIsCustom = $derived(
    customMode || !server.hostAddresses.includes(server.settings.host),
  );
  const hostSelectValue = $derived(
    hostIsCustom ? HOST_CUSTOM : server.settings.host,
  );
  let hostSelect: HTMLSelectElement | null = $state(null);
  // A plain `value` attribute on <select> is not reactive; keep the native
  // control in sync with the derived choice after a rollback or a save.
  $effect(() => {
    const value = hostSelectValue;
    if (hostSelect) hostSelect.value = value;
  });
  const address = $derived(
    running && server.status.address
      ? server.status.address
      : `${server.settings.host}:${server.settings.port}`,
  );
  const error = $derived(server.error ?? server.status.lastError);
  const statusText = $derived(statusLabel(serverState));
  const actionText = $derived(
    serverState === "running"
      ? t("action.stop")
      : serverState === "starting"
        ? t("action.starting")
        : serverState === "stopping"
          ? t("action.stopping")
          : t("action.start"),
  );
  const footerHint = $derived(
    serverState === "stopped" ? t("main.hintReady") : t("main.hintLocked"),
  );

  async function toggleServer(): Promise<void> {
    if (running) {
      await server.stop();
    } else if (serverState === "stopped") {
      await server.start();
    }
  }

  function persist(): void {
    void server.persist();
  }

  /** Enumerated choices save immediately; the custom entry opens its input. */
  function onHostSelect(event: Event): void {
    const value = (event.currentTarget as HTMLSelectElement).value;
    if (value === HOST_CUSTOM) {
      customMode = true;
      return;
    }
    customMode = false;
    server.settings.host = value;
    persist();
  }

  /** Persist a custom hostname/IP on change and return to the dropdown when
   * the saved (or rolled-back) value is enumerated again. */
  async function onCustomHostChange(): Promise<void> {
    await server.persist();
    if (server.hostAddresses.includes(server.settings.host)) {
      customMode = false;
    }
  }

  function reset(): void {
    customMode = false;
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

    <ErrorLine message={error} />
    <ErrorLine message={server.saveError} />
    {#if server.status.v2.state === "error"}
      <p class="v2-warning">
        {t("pairing.v2Unavailable", { message: server.status.v2.lastError ?? "" })}
      </p>
    {/if}

    <div class="server-reset pairing-entry">
      <div class="server-reset-text">
        <div class="server-reset-label">{t("pairing.title")}</div>
        <div class="sub">{t("pairing.entryHint")}</div>
      </div>
      <button class="link" onclick={() => server.openPairing()}>{t("pairing.open")}</button>
    </div>

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
            <select
              id="host"
              bind:this={hostSelect}
              disabled={locked}
              onchange={onHostSelect}
            >
              {#each server.hostAddresses as address (address)}
                <option value={address}>{address}</option>
              {/each}
              <option value={HOST_CUSTOM}>{t("field.hostCustomOption")}</option>
            </select>
            {#if hostIsCustom}
              <input
                id="host-custom"
                type="text"
                bind:value={server.settings.host}
                disabled={locked}
                aria-label={t("field.hostCustomLabel")}
                aria-describedby="host-custom-hint"
                onchange={onCustomHostChange}
              />
              <p class="sub" id="host-custom-hint">
                {t("field.hostCustomHint")}
              </p>
            {/if}
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
