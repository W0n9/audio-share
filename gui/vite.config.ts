import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Tauri expects the dev server on a fixed port and must see its own logs.
export default defineConfig({
  plugins: [svelte()],
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
