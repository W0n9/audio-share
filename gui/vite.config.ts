import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { defineConfig, type Plugin } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

/**
 * styles.css deliberately keeps two identical dark token blocks: the
 * `prefers-color-scheme` fallback for the moment before theme.ts runs, and
 * the authoritative `[data-theme="dark"]` block. Only a comment asks them to
 * stay identical, so this hook re-reads the stylesheet at the end of every
 * build and fails it once the two blocks drift apart.
 */
function darkTokenParity(): Plugin {
  return {
    name: "dark-token-parity",
    apply: "build",
    closeBundle() {
      const css = readFileSync(
        fileURLToPath(new URL("./src/styles.css", import.meta.url)),
        "utf8",
      );
      const fallback = css.match(
        /@media \(prefers-color-scheme: dark\) \{\s*:root:not\(\[data-theme="light"\]\) \{([^}]*)\}/,
      );
      const authoritative = css.match(/:root\[data-theme="dark"\] \{([^}]*)\}/);
      if (!fallback || !authoritative) {
        throw new Error(
          "dark-token-parity: expected both dark token blocks in src/styles.css, found " +
            (fallback ? "only the authoritative" : authoritative ? "only the fallback" : "neither"),
        );
      }
      const tokens = (body: string): string =>
        body
          .split(";")
          .map((line) => line.trim())
          .filter(Boolean)
          .sort()
          .join(";\n");
      const fallbackTokens = tokens(fallback[1]);
      const authoritativeTokens = tokens(authoritative[1]);
      if (fallbackTokens !== authoritativeTokens) {
        throw new Error(
          "dark-token-parity: the prefers-color-scheme fallback and the " +
            '[data-theme="dark"] block in src/styles.css have diverged\n' +
            `  fallback: ${fallbackTokens}\n` +
            `  dark:     ${authoritativeTokens}`,
        );
      }
    },
  };
}

// Tauri expects the dev server on a fixed port and must see its own logs.
export default defineConfig({
  plugins: [svelte(), darkTokenParity()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      // Rust sources are watched by `tauri dev`, not Vite.
      ignored: ["**/src-tauri/**"],
    },
  },
  build: {
    // WebView2 on Windows 10 and WebKitGTK 4.1 on Ubuntu 24.04 handle ES2021.
    target: "es2021",
  },
});
