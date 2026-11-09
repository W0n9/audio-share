import type { Encoding } from "./ipc";
import { t } from "./i18n.svelte";

/**
 * The frozen v0.3.4 encoding choices, in display order. Both the main view and
 * the settings view use this list so they cannot drift.
 */
export const encodings: readonly Encoding[] = [
  "default",
  "f32",
  "s8",
  "s16",
  "s24",
  "s32",
];

/** Display label for one encoding; only `default` is translated. */
export function encodingLabel(value: Encoding): string {
  return value === "default" ? t("encoding.default") : value;
}
