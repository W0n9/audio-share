<script lang="ts">
  /**
   * The shared inline SVG icon set for the Server GUI, replacing the
   * character icons (`♪`, `⚙`, `←`). Paths follow the Variant C design
   * (prototype/server-gui-modernization @ 604f0d5); stroke icons inherit
   * `currentColor` so tokens drive their color.
   */
  interface Props {
    name: "audio" | "back" | "chevron" | "globe" | "music" | "play" | "settings" | "stop";
    size?: number;
  }

  let { name, size = 18 }: Props = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="1.8"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  {#if name === "settings"}
    <path d="M4 21v-7M4 10V3M12 21v-9M12 8V3M20 21v-5M20 12V3" />
    <path d="M1 14h6M9 8h6M17 16h6" />
  {:else if name === "back"}
    <path d="M19 12H5M12 19l-7-7 7-7" />
  {:else if name === "play"}
    <path d="m8 5 11 7-11 7Z" fill="currentColor" stroke="none" />
  {:else if name === "stop"}
    <rect x="6" y="6" width="12" height="12" rx="2" fill="currentColor" stroke="none" />
  {:else if name === "chevron"}
    <path d="m6 9 6 6 6-6" />
  {:else if name === "audio"}
    <path d="M11 5 6 9H2v6h4l5 4Z" />
    <path d="M15.5 8.5a5 5 0 0 1 0 7M18.5 5.5a9 9 0 0 1 0 13" />
  {:else if name === "globe"}
    <circle cx="12" cy="12" r="9" />
    <path d="M3 12h18M12 3a14 14 0 0 1 0 18M12 3a14 14 0 0 0 0 18" />
  {:else if name === "music"}
    <path d="M9 18V5l12-2v13" />
    <circle cx="6" cy="18" r="3" />
    <circle cx="18" cy="16" r="3" />
  {/if}
</svg>
