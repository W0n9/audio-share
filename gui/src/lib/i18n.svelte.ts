import enUS from "../locales/en-US.json";
import zhCN from "../locales/zh-CN.json";
import type { Locale } from "./ipc";

/**
 * Interface language. The Rust bootstrap resolves `[App] language=default`
 * against the system locale before the shell mounts; the UI never switches
 * live (changing the setting shows a restart hint).
 */
export type { Locale };

export const locales: ReadonlyArray<{ id: Locale; label: string }> = [
  { id: "en-US", label: "English" },
  { id: "zh-CN", label: "简体中文" },
];

const resources: Record<Locale, Record<string, string>> = {
  "en-US": enUS,
  "zh-CN": zhCN,
};

let current = $state<Locale>("en-US");

/** Switch the active locale (startup only, per spec). */
export function setLocale(locale: Locale): void {
  current = locale;
}

/**
 * The system-locale fallback used only if the Rust bootstrap is unavailable.
 * Rust performs the authoritative resolution.
 */
export function systemLocale(): Locale {
  const candidates = typeof navigator === "undefined" ? [] : navigator.languages ?? [];
  for (const candidate of candidates) {
    if (candidate.toLowerCase().startsWith("zh")) return "zh-CN";
  }
  return "en-US";
}

/** Translate a flat dotted key, interpolating `{name}` parameters. */
export function t(key: string, params?: Record<string, string | number>): string {
  const template = resources[current][key] ?? resources["en-US"][key] ?? key;
  if (!params) return template;
  return template.replace(/\{(\w+)\}/g, (_, name: string) =>
    name in params ? String(params[name]) : `{${name}}`,
  );
}
