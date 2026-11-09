import enUS from "../locales/en-US.json";

/**
 * Interface language. Ticket 10 adds zh-CN and startup persistence; this module
 * is the extension point, and `en-US.json` is shared with the Rust tray labels.
 */
export type Locale = "en-US";

export const locales: ReadonlyArray<{ id: Locale; label: string }> = [
  { id: "en-US", label: "English" },
];

const resources: Record<Locale, Record<string, string>> = {
  "en-US": enUS,
};

let current = $state<Locale>("en-US");

/** Switch the active locale (ticket 10 applies this at startup only). */
export function setLocale(locale: Locale): void {
  current = locale;
}

/** The active locale. */
export function getLocale(): Locale {
  return current;
}

/** Translate a flat dotted key, interpolating `{name}` parameters. */
export function t(key: string, params?: Record<string, string | number>): string {
  const template = resources[current][key] ?? resources["en-US"][key] ?? key;
  if (!params) return template;
  return template.replace(/\{(\w+)\}/g, (_, name: string) =>
    name in params ? String(params[name]) : `{${name}}`,
  );
}
