import { mount } from "svelte";
import "./styles.css";
import App from "./App.svelte";
import { api } from "./lib/ipc";
import { setLocale, systemLocale } from "./lib/i18n.svelte";
import { applyTheme } from "./lib/theme";
import { server } from "./lib/store.svelte";

/**
 * Resolve the configured language and theme before the first render so the
 * shell and the native tray share one language and no wrong-theme flash is
 * possible.
 */
async function boot(): Promise<void> {
  try {
    const data = await api.getBootstrap();
    setLocale(data.locale);
    server.hydrate(data);
    applyTheme(data.settings.theme);
  } catch (error) {
    setLocale(systemLocale());
    applyTheme("system");
    server.error = String(error);
  }
  mount(App, { target: document.getElementById("app")! });
}

void boot();
