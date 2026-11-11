<script lang="ts">
  /**
   * Shared single-line error rendering. Renders the message with
   * role="alert" only when one exists, so a view can pass its nullable
   * store field straight through and keep the previous truthiness gate.
   */
  let { message }: { message: string | null } = $props();
</script>

{#if message}
  <p class="error" role="alert">{message}</p>
{/if}
