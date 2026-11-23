<script lang="ts">
  import { server } from "../lib/store.svelte";
  import { t } from "../lib/i18n.svelte";
  import ErrorLine from "../lib/ErrorLine.svelte";
  import { tick, untrack } from "svelte";
  import type {
    PairingInvitation,
    PairingLiveInfo,
    PairingMode,
    PairingRequestInfo,
  } from "../lib/ipc";

  const v2 = $derived(server.status.v2);
  const v2Ready = $derived(v2.state === "running");
  const invitation = $derived(server.pairing?.invitation ?? null);
  const live = $derived(server.pairing?.live ?? []);
  const pending = $derived(server.pairing?.pending ?? []);

  let body: HTMLDivElement | undefined = $state();
  let invitationEl: HTMLDivElement | undefined = $state();
  let dialogEl: HTMLDivElement | undefined = $state();

  /** Destructive action awaiting the in-app confirmation dialog. */
  type ConfirmTarget = {
    kind: "revoke" | "delete";
    clientId: string;
    displayName: string;
  };
  let confirmTarget = $state<ConfirmTarget | null>(null);
  let confirmTrigger: HTMLElement | null = null;
  let confirmCancelEl: HTMLButtonElement | undefined = $state();
  let confirmActionEl: HTMLButtonElement | undefined = $state();

  // Closing the pairing dialog must also drop a confirmation that was left
  // open, otherwise it would reappear the next time pairing opens.
  $effect(() => {
    if (!server.pairingOpen) {
      untrack(() => {
        confirmTarget = null;
        confirmTrigger = null;
      });
    }
  });

  function begin(mode: PairingMode): void {
    void server.beginPairing(mode);
  }

  function cancel(): void {
    void server.cancelPairing();
  }

  function refresh(): void {
    if (invitation) void server.beginPairing(invitation.mode, true);
  }

  function usePairingCode(): void {
    void server.beginPairing("sas", true);
  }

  /** Start a fresh invitation for a revoked device; scroll it into view. */
  async function rePair(clientId: string, displayName: string): Promise<void> {
    await server.repairClient(clientId, displayName);
    await tick();
    // Pending/live cards render above the invitation, so scroll to the
    // invitation itself rather than assuming it is at the top.
    if (invitationEl) {
      invitationEl.scrollIntoView({ block: "nearest", behavior: "smooth" });
    } else {
      body?.scrollTo({ top: 0, behavior: "smooth" });
    }
  }

  function remove(
    clientId: string,
    displayName: string,
    trigger: HTMLElement | null,
  ): void {
    openConfirm({ kind: "delete", clientId, displayName }, trigger);
  }

  /**
   * A revoked identity's card is the explicit Restore confirmation (ADR-0006):
   * its button must carry the Restore decision, never a plain Accept. Every
   * other card is an ordinary approve.
   */
  function decide(request: PairingLiveInfo | PairingRequestInfo): void {
    if (request.restoreCandidate) {
      void server.restorePairing(request.approvalId);
    } else {
      void server.approvePairing(request.approvalId);
    }
  }

  function decline(approvalId: number): void {
    void server.declinePairing(approvalId);
  }

  function revoke(
    clientId: string,
    displayName: string,
    trigger: HTMLElement | null,
  ): void {
    openConfirm({ kind: "revoke", clientId, displayName }, trigger);
  }

  function openConfirm(target: ConfirmTarget, trigger: HTMLElement | null): void {
    confirmTrigger = trigger;
    confirmTarget = target;
    // Move focus into the dialog once it has rendered. Cancel is the safe
    // default for a destructive action.
    void tick().then(() => confirmCancelEl?.focus());
  }

  /** Close without acting; focus returns to the button that opened it. */
  function closeConfirm(): void {
    const trigger = confirmTrigger;
    confirmTarget = null;
    confirmTrigger = null;
    void tick().then(() => {
      if (trigger?.isConnected) trigger.focus();
      else dialogEl?.focus();
    });
  }

  /** Only an explicit confirmation runs the destructive action. */
  function applyConfirm(): void {
    const target = confirmTarget;
    if (!target) return;
    const trigger = confirmTrigger;
    confirmTarget = null;
    confirmTrigger = null;
    if (target.kind === "revoke") void server.revokeClient(target.clientId);
    else void server.deleteClient(target.clientId);
    // The row may disappear (delete) or lose the pressed button (revoke), so
    // fall back to the pairing dialog when the trigger is no longer rendered.
    void tick().then(() => {
      if (trigger?.isConnected) trigger.focus();
      else dialogEl?.focus();
    });
  }

  /**
   * Escape cancels; Tab stays inside the two actions. The background pairing
   * dialog is `inert` while this dialog is open, so it cannot take focus.
   */
  function onConfirmKeydown(event: KeyboardEvent): void {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      closeConfirm();
      return;
    }
    if (event.key !== "Tab") return;
    const focusable = [confirmCancelEl, confirmActionEl].filter(
      (el): el is HTMLButtonElement => el !== undefined && !el.disabled,
    );
    if (focusable.length === 0) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }

  /**
   * Escape still cancels if a backdrop click moved focus out of the dialog.
   * The dialog handler stops propagation while focus is inside it.
   */
  function onWindowKeydown(event: KeyboardEvent): void {
    if (event.key !== "Escape" || !confirmTarget) return;
    event.preventDefault();
    closeConfirm();
  }

  /**
   * Saved name for a connection opened from a Re-pair row, or one the core
   * flagged as a restore candidate. Used only for the "Restoring…" label.
   */
  function restoreName(request: {
    clientId: string;
    restoreCandidate: string | null;
  }): string | null {
    // Only a real restore candidate gets the Restoring label; a Re-pair
    // invitation alone must not imply that an active/new identity is restored.
    if (!request.restoreCandidate) return null;
    if (server.repairTarget?.clientId === request.clientId) {
      return server.repairTarget.displayName;
    }
    return request.restoreCandidate;
  }

  function countdown(seconds: number): string {
    const total = Math.max(0, Math.floor(seconds));
    return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
  }
</script>

<svelte:window onkeydown={onWindowKeydown} />

<!-- One card for a live or verified pairing. The only differences are the
     live hint (no actions yet) and the approve/decline footer. -->
{#snippet requestCard(
  request: PairingLiveInfo | PairingRequestInfo,
  live: boolean,
)}
  {@const label = restoreName(request)}
  <div class="pairing-card" class:pairing-card-live={live}>
    {#if request.restoreCandidate}
      <div class="pairing-card-title">{t("pairing.restoreTitle")}</div>
      <div class="pairing-card-device">{request.displayName}</div>
      <p class="pairing-hint">{t("pairing.restoreBody")}</p>
    {:else}
      <div class="pairing-card-title">{request.displayName}</div>
    {/if}
    {#if label}
      <p class="pairing-restore-name">
        {t("pairing.restoring", { name: label })}
      </p>
    {/if}
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
    {#if live}
      <p class="pairing-hint">{t("pairing.liveHint")}</p>
    {:else}
      <div class="pairing-card-actions">
        <button
          class="pairing-primary"
          disabled={server.pairingBusy}
          onclick={() => decide(request)}
        >
          {request.restoreCandidate
            ? t("pairing.restoreAccept")
            : t("pairing.accept")}
        </button>
        <button
          class="pairing-secondary"
          disabled={server.pairingBusy}
          onclick={() => decline(request.approvalId)}
        >
          {t("pairing.decline")}
        </button>
      </div>
    {/if}
  </div>
{/snippet}

<!-- Invitation details, rendered below pending/live cards so an approval
     request is what the operator sees first when the window is small. -->
{#snippet invitationBlock(inv: PairingInvitation)}
  <div class="pairing-invitation" bind:this={invitationEl}>
    {#if inv.state === "active" && inv.qrSvg}
      <div class="pairing-qr" aria-label={t("pairing.qrAlt")}>
        {@html inv.qrSvg}
      </div>
    {/if}
    <div class="pairing-row">
      <span>{t("pairing.mode")}</span>
      <strong>
        {inv.mode === "qr" ? t("pairing.modeQr") : t("pairing.modeSas")}
      </strong>
    </div>
    {#if inv.state === "active" && inv.qrUri}
      <div class="pairing-row pairing-row-stack">
        <span>{t("pairing.uri")}</span>
        <code class="pairing-uri">{inv.qrUri}</code>
      </div>
    {/if}
    <div class="pairing-row pairing-row-stack">
      <span>{t("pairing.fingerprint")}</span>
      <code class="pairing-fingerprint">{inv.fingerprint}</code>
    </div>
    {#if inv.state === "active"}
      <div class="pairing-row">
        <span>{t("pairing.expires")}</span>
        <strong class="pairing-countdown">{countdown(inv.expiresInSecs)}</strong>
      </div>
      <p class="pairing-hint">{t("pairing.sasHint")}</p>
      <!-- QR mode also admits SAS in the same operator-opened window
           (the admin begin(Qr) path opens both), so a Re-pair row can
           switch to the pairing code without losing the invitation. -->
      {#if inv.mode === "qr"}
        <button
          class="pairing-secondary"
          disabled={!v2Ready || server.pairingBusy}
          onclick={usePairingCode}
        >
          {t("pairing.startSas")}
        </button>
      {/if}
    {:else}
      <p class="pairing-error">
        {inv.state === "consumed"
          ? t("pairing.qrConsumed")
          : t("pairing.qrExpired")}
      </p>
      <div class="pairing-actions">
        <button
          class="pairing-primary"
          disabled={server.pairingBusy}
          onclick={refresh}
        >
          {t("pairing.refreshQr")}
        </button>
        <button
          class="pairing-secondary"
          disabled={!v2Ready || server.pairingBusy}
          onclick={usePairingCode}
        >
          {t("pairing.startSas")}
        </button>
      </div>
    {/if}
    <button class="pairing-secondary" onclick={cancel}>
      {t("pairing.cancel")}
    </button>
  </div>
{/snippet}

{#if server.pairingOpen}
  <div class="pairing-backdrop" role="presentation">
    <div
      class="pairing-dialog"
      role="dialog"
      aria-modal="true"
      aria-labelledby="pairing-title"
      bind:this={dialogEl}
      tabindex="-1"
      inert={confirmTarget !== null}
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

      <div class="pairing-body" bind:this={body}>
        {#if v2.state === "error"}
          <p class="pairing-error">
            {t("pairing.v2Unavailable", { message: v2.lastError ?? "" })}
          </p>
        {:else if !v2Ready}
          <p class="pairing-hint">{t("pairing.needsServer")}</p>
        {/if}

        <ErrorLine message={server.pairingError} />

        {#if server.pairedName}
          <p class="pairing-success" role="status">
            {t("pairing.paired", { name: server.pairedName })}
          </p>
        {/if}

        <!-- Approvals and live connections come before the invitation so the
             operator's next action stays above the fold at the minimum window
             size (380x560). With no pending/live request the invitation is
             still the first block, keeping the QR fully scannable. -->
        {#if pending.length > 0}
          <div class="pairing-section-title">{t("pairing.pendingTitle")}</div>
          {#each pending as request (request.approvalId)}
            {@render requestCard(request, false)}
          {/each}
        {/if}

        {#if live.length > 0}
          <div class="pairing-section-title">{t("pairing.liveTitle")}</div>
          {#each live as request (request.approvalId)}
            {@render requestCard(request, true)}
          {/each}
          <!-- A consumed QR hides the invitation block; keep Cancel reachable
               while a live connection waits for the phone's confirmation. -->
          {#if !invitation || invitation.state !== "active"}
            <button class="pairing-secondary" onclick={cancel}>
              {t("pairing.cancel")}
            </button>
          {/if}
        {/if}

        {#if invitation && (invitation.state === "active" || invitation.mode === "qr")}
          {#if invitation.state === "active" || !(live.length > 0 || pending.length > 0 || server.pairingInFlight || server.pairingSucceeded)}
            {@render invitationBlock(invitation)}
          {/if}
        {/if}

        <!-- After a successful pairing the QR is consumed and no invitation
             block remains; offer a fresh start instead of a dead end. -->
        {#if live.length === 0 && ((server.pairingSucceeded && invitation?.state !== "active") || !invitation || (invitation.mode === "sas" && invitation.state !== "active"))}
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
              <div class="pairing-device-actions">
                {#if client.state === "revoked"}
                  <button
                    class="pairing-link"
                    disabled={server.pairingBusy}
                    onclick={() => rePair(client.clientId, client.displayName)}
                  >
                    {t("pairing.rePair")}
                  </button>
                  <button
                    class="pairing-link danger"
                    disabled={server.deletingClientId !== null}
                    onclick={(event) =>
                      remove(client.clientId, client.displayName, event.currentTarget)}
                  >
                    {t("pairing.delete")}
                  </button>
                {:else}
                  <button
                    class="pairing-link danger"
                    disabled={server.revokingClientId !== null}
                    onclick={(event) =>
                      revoke(client.clientId, client.displayName, event.currentTarget)}
                  >
                    {t("pairing.revoke")}
                  </button>
                {/if}
              </div>
            </div>
          {/each}
        {/if}
      </div>
    </div>
  </div>

  <!-- Destructive confirmation. Same backdrop, surface, radius, and type as
       the pairing dialog; a second layer only because it stacks above it. -->
  {#if confirmTarget}
    <div
      class="pairing-backdrop pairing-backdrop-stacked"
      role="presentation"
    >
      <div
        class="pairing-dialog pairing-confirm"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="pairing-confirm-title"
        aria-describedby="pairing-confirm-body"
        onkeydown={onConfirmKeydown}
        tabindex="-1"
      >
        <header class="pairing-header">
          <h2 id="pairing-confirm-title">
            {confirmTarget.kind === "revoke"
              ? t("pairing.confirmRevokeTitle")
              : t("pairing.confirmDeleteTitle")}
          </h2>
        </header>
        <div class="pairing-body">
          <p class="pairing-confirm-body" id="pairing-confirm-body">
            {confirmTarget.kind === "revoke"
              ? t("pairing.confirmRevoke", { name: confirmTarget.displayName })
              : t("pairing.confirmDelete", { name: confirmTarget.displayName })}
          </p>
          <div class="pairing-actions pairing-confirm-actions">
            <button
              class="pairing-secondary"
              bind:this={confirmCancelEl}
              onclick={closeConfirm}
            >
              {t("action.cancel")}
            </button>
            <button
              class="pairing-primary danger"
              bind:this={confirmActionEl}
              disabled={confirmTarget.kind === "revoke"
                ? server.revokingClientId !== null
                : server.deletingClientId !== null}
              onclick={applyConfirm}
            >
              {confirmTarget.kind === "revoke"
                ? t("pairing.revoke")
                : t("pairing.delete")}
            </button>
          </div>
        </div>
      </div>
    </div>
  {/if}
{/if}
