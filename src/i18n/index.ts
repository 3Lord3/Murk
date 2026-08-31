// Interface languages. English is the source: every key is written here
// first, and the other catalogues are translations of it, so someone who does
// not read Russian can still contribute a language.
import { createI18n } from "vue-i18n";
import { invoke } from "@tauri-apps/api/core";

import en from "../locales/en.json";
import ru from "../locales/ru.json";

export const FALLBACK_LOCALE = "en";

/** Locale code -> the language's own name for itself. Never translated. */
export const LOCALE_NAMES = {
  en: "English",
  ru: "Русский",
} as const satisfies Record<string, string>;

export type Locale = keyof typeof LOCALE_NAMES;

function isLocale(tag: string): tag is Locale {
  return tag in LOCALE_NAMES;
}
/** `"system"` means "follow the system and keep following it". */
export type LocaleSetting = "system" | Locale;

export const i18n = createI18n({
  legacy: false,
  locale: FALLBACK_LOCALE,
  fallbackLocale: FALLBACK_LOCALE,
  messages: { en, ru },
});

/**
 * The system's languages as the backend read them from the environment, most
 * preferred first. Filled once at startup, because `navigator.languages` is
 * not usable here: WebKitGTK reports `en-US` whatever the session's locale is,
 * which showed a Russian desktop an English interface.
 */
let systemTags: readonly string[] | null = null;

/**
 * The best of the system's languages we have a catalogue for. A regional tag
 * matches its base language, so `ru-RU` finds `ru`.
 */
export function systemLocale(): Locale {
  const preferred =
    systemTags ??
    (navigator.languages?.length ? navigator.languages : [navigator.language]);
  for (const tag of preferred) {
    const base = tag.toLowerCase().split("-")[0];
    if (base && isLocale(base)) return base;
  }
  return FALLBACK_LOCALE;
}

export function resolveLocale(setting: LocaleSetting): Locale {
  return setting === "system" ? systemLocale() : setting;
}

function apply(setting: LocaleSetting) {
  const locale = resolveLocale(setting);
  i18n.global.locale.value = locale;
  document.documentElement.lang = locale;
}

/**
 * Read the stored choice and apply it. Called before the app is mounted, so
 * the first frame is already in the right language.
 */
export async function initLocale(): Promise<LocaleSetting> {
  let setting: LocaleSetting = "system";
  try {
    const tags = await invoke<string[]>("system_languages");
    if (tags.length) systemTags = tags;
  } catch {
    // Without the environment we still have navigator's guess.
  }
  try {
    setting = (await invoke<string>("get_locale")) as LocaleSetting;
  } catch {
    // A missing setting is not worth refusing to start over.
  }
  if (setting !== "system" && !isLocale(setting)) setting = "system";
  apply(setting);
  return setting;
}

export async function setLocale(setting: LocaleSetting): Promise<void> {
  apply(setting);
  await invoke("set_locale", { locale: setting });
}
