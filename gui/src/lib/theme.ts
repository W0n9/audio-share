import type { Theme } from "./ipc";

/**
 * Theme application for the Server GUI.
 *
 * The persisted mode (`system`, `light`, or `dark`) is resolved against the
 * operating system color scheme and applied to the document element before
 * the shell mounts, mirroring the language timing in `main.ts`. While the
 * mode is `system`, later OS scheme changes re-apply automatically; the CSS
 * `prefers-color-scheme` fallback in `styles.css` only covers the moment
 * before this module runs.
 */

const DARK_QUERY = "(prefers-color-scheme: dark)";

let media: MediaQueryList | null = null;
let onSystemChange: ((event: MediaQueryListEvent) => void) | null = null;

function systemPrefersDark(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof window.matchMedia === "function" &&
    window.matchMedia(DARK_QUERY).matches
  );
}

function setDocumentTheme(theme: "light" | "dark"): void {
  document.documentElement.dataset.theme = theme;
}

function stopSystemWatch(): void {
  if (media && onSystemChange) media.removeEventListener("change", onSystemChange);
  media = null;
  onSystemChange = null;
}

function watchSystem(): void {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") return;
  media = window.matchMedia(DARK_QUERY);
  onSystemChange = (event: MediaQueryListEvent): void => {
    setDocumentTheme(event.matches ? "dark" : "light");
  };
  media.addEventListener("change", onSystemChange);
}

/**
 * Apply `mode` to the document immediately. Call before the shell mounts and
 * again whenever the persisted mode changes; only `system` keeps watching the
 * operating system.
 */
export function applyTheme(mode: Theme): void {
  stopSystemWatch();
  if (mode === "system") watchSystem();
  setDocumentTheme(mode === "dark" || (mode === "system" && systemPrefersDark()) ? "dark" : "light");
}
