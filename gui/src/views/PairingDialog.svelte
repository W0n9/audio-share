<script lang="ts">
  import { server } from "../lib/store.svelte";
  import { t } from "../lib/i18n.svelte";
  import ErrorLine from "../lib/ErrorLine.svelte";
  import type { PairingMode } from "../lib/ipc";

  const v2 = $derived(server.status.v2);
  const v2Ready = $derived(v2.state === "running");
  const invitation = $derived(server.pairing?.invitation ?? null);
  const live = $derived(server.pairing?.live ?? []);
  const pending = $derived(server.pairing?.pending ?? []);

  function begin(mode: PairingMode): void {
    void server.beginPairing(mode);
  }

  function cancel(): void {
    void server.cancelPairing();
  }

  function refresh(): void {
    if (invitation) void server.beginPairing(invitation.mode);
  }

  function approve(approvalId: number): void {
    void server.approvePairing(approvalId);
  }

  function decline(approvalId: number): void {
    void server.declinePairing(approvalId);
  }

  function revoke(clientId: string, displayName: string): void {
    if (window.confirm(t("pairing.confirmRevoke", { name: displayName }))) {
      void server.revokeClient(clientId);
    }
  }

  function countdown(seconds: number): string {
    const total = Math.max(0, Math.floor(seconds));
    return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
  }
</script>

{#if server.pairingOpen}
  <div class="pairing-backdrop" role="presentation">
    <div
      class="pairing-dialog"
      role="dialog"
      aria-modal="true"
      aria-labelledby="pairing-title"
    >
      <header class="pairing-header">
        <h2 id="pairing-title">{t("pairing.title")}</h2>
        <button
          class="pairing-close"
          aria-label={t("action.close")}
          onclick={() => server.closePairing()}
        >
          ✕
        </button>
      </header>

      <div class="pairing-body">
        {#if v2.state === "error"}
          <p class="pairing-error">
            {t("pairing.v2Unavailable", { message: v2.lastError ?? "" })}
          </p>
        {:else if !v2Ready}
          <p class="pairing-hint">{t("pairing.needsServer")}</p>
        {/if}

        <ErrorLine message={server.pairingError} />

        {#if invitation && (invitation.state === "active" || invitation.mode === "qr")}
          <div class="pairing-invitation">
            {#if invitation.state === "active" && invitation.qrSvg}
              <div class="pairing-qr" aria-label={t("pairing.qrAlt")}>
                {@html invitation.qrSvg}
              </div>
            {/if}
            <div class="pairing-row">
              <span>{t("pairing.mode")}</span>
              <strong>
                {invitation.mode === "qr" ? t("pairing.modeQr") : t("pairing.modeSas")}
              </strong>
            </div>
            {#if invitation.state === "active" && invitation.qrUri}
              <div class="pairing-row pairing-row-stack">
                <span>{t("pairing.uri")}</span>
                <code class="pairing-uri">{invitation.qrUri}</code>
              </div>
            {/if}
            <div class="pairing-row pairing-row-stack">
              <span>{t("pairing.fingerprint")}</span>
              <code class="pairing-fingerprint">{invitation.fingerprint}</code>
            </div>
            {#if invitation.state === "active"}
              <div class="pairing-row">
                <span>{t("pairing.expires")}</span>
                <strong class="pairing-countdown">{countdown(invitation.expiresInSecs)}</strong>
              </div>
              <p class="pairing-hint">{t("pairing.sasHint")}</p>
            {:else}
              <p class="pairing-error">
                {invitation.state === "consumed"
                  ? t("pairing.qrConsumed")
                  : t("pairing.qrExpired")}
              </p>
              <button
                class="pairing-primary"
                disabled={server.pairingBusy}
                onclick={refresh}
              >
                {t("pairing.refreshQr")}
              </button>
            {/if}
            <button class="pairing-secondary" onclick={cancel}>
              {t("pairing.cancel")}
            </button>
          </div>
        {/if}

        {#if live.length > 0}
          <div class="pairing-section-title">{t("pairing.liveTitle")}</div>
          {#each live as request (request.approvalId)}
            <div class="pairing-card pairing-card-live">
              <div class="pairing-card-title">{request.displayName}</div>
              <div class="pairing-row">
                <span>{t("pairing.clientId")}</span>
                <code>{request.clientId}</code>
              </div>
              <div class="pairing-row">
                <span>{t("pairing.address")}</span>
                <code>{request.address}</code>
              </div>
              <div class="pairing-code">
                <span>{t("pairing.pairingCode")}</span>
                <strong>{request.sas ?? t("pairing.codeUnavailable")}</strong>
              </div>
              <p class="pairing-hint">{t("pairing.liveHint")}</p>
            </div>
          {/each}
          {#if !invitation || (invitation.mode === "sas" && invitation.state !== "active")}
            <button class="pairing-secondary" onclick={cancel}>
              {t("pairing.cancel")}
            </button>
          {/if}
        {/if}

        {#if (!invitation || (invitation.mode === "sas" && invitation.state !== "active")) && live.length === 0}
          <div class="pairing-actions">
            <button
              class="pairing-primary"
              disabled={!v2Ready || server.pairingBusy}
              onclick={() => begin("qr")}
            >
              {t("pairing.startQr")}
            </button>
            <button
              class="pairing-secondary"
              disabled={!v2Ready || server.pairingBusy}
              onclick={() => begin("sas")}
            >
              {t("pairing.startSas")}
            </button>
          </div>
          <p class="pairing-hint">{t("pairing.sasHint")}</p>
        {/if}

        {#if pending.length > 0}
          <div class="pairing-section-title">{t("pairing.pendingTitle")}</div>
          {#each pending as request (request.approvalId)}
            <div class="pairing-card">
              <div class="pairing-card-title">{request.displayName}</div>
              <div class="pairing-row">
                <span>{t("pairing.clientId")}</span>
                <code>{request.clientId}</code>
              </div>
              <div class="pairing-row">
                <span>{t("pairing.address")}</span>
                <code>{request.address}</code>
              </div>
              <div class="pairing-code">
                <span>{t("pairing.pairingCode")}</span>
                <strong>{request.sas ?? t("pairing.codeUnavailable")}</strong>
              </div>
              <div class="pairing-card-actions">
                <button
                  class="pairing-primary"
                  disabled={server.pairingBusy}
                  onclick={() => approve(request.approvalId)}
                >
                  {t("pairing.accept")}
                </button>
                <button
                  class="pairing-secondary"
                  disabled={server.pairingBusy}
                  onclick={() => decline(request.approvalId)}
                >
                  {t("pairing.decline")}
                </button>
              </div>
            </div>
          {/each}
        {/if}

        <div class="pairing-section-title">{t("pairing.devicesTitle")}</div>
        {#if server.clients.length === 0}
          <p class="pairing-hint">{t("pairing.noDevices")}</p>
        {:else}
          {#each server.clients as client (client.clientId)}
            <div class="pairing-device">
              <div class="pairing-device-main">
                <strong>{client.displayName}</strong>
                <code>{client.clientId}</code>
              </div>
              <span
                class="pairing-state"
                class:is-revoked={client.state === "revoked"}
              >
                {client.state === "revoked" ? t("pairing.revoked") : t("pairing.active")}
              </span>
              {#if client.state !== "revoked"}
                <button
                  class="pairing-link danger"
                  disabled={server.revokingClientId !== null}
                  onclick={() => revoke(client.clientId, client.displayName)}
                >
                  {t("pairing.revoke")}
                </button>
              {/if}
            </div>
          {/each}
        {/if}
      </div>
    </div>
  </div>
{/if}
