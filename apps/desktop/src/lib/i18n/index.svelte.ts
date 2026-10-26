import { de, en, es, fr, it, ja, pt, type Dictionary } from "./messages";

export type Locale = "es" | "en" | "pt" | "fr" | "de" | "it" | "ja";

export const LOCALES: Array<{ id: Locale; label: string }> = [
  { id: "es", label: "ES" },
  { id: "en", label: "EN" },
  { id: "pt", label: "PT" },
  { id: "fr", label: "FR" },
  { id: "de", label: "DE" },
  { id: "it", label: "IT" },
  { id: "ja", label: "JA" },
];

const STORAGE_KEY = "dvdforge_locale";
const supported = new Set<Locale>(LOCALES.map((locale) => locale.id));

function normalizeLocale(value: string | null | undefined): Locale | null {
  const prefix = (value ?? "").trim().toLowerCase().split(/[-_]/)[0];
  return supported.has(prefix as Locale) ? (prefix as Locale) : null;
}

function initialLocale(): Locale {
  if (typeof window === "undefined") return "es";
  try {
    const saved = normalizeLocale(window.localStorage.getItem(STORAGE_KEY));
    if (saved) return saved;
  } catch {
    // Storage can be disabled by the webview; keep detecting.
  }
  const detected = normalizeLocale(typeof navigator === "undefined" ? null : navigator.language);
  return detected ?? "en";
}

class I18n {
  locale = $state<Locale>(initialLocale());

  constructor() {
    if (typeof document !== "undefined") {
      document.documentElement.lang = this.locale;
    }
  }

  t(key: string, values?: Record<string, string | number>): string {
    const dictionary: Dictionary =
      this.locale === "es"
        ? es
        : this.locale === "en"
          ? en
          : this.locale === "pt"
            ? pt
            : this.locale === "fr"
              ? fr
              : this.locale === "de"
                ? de
                : this.locale === "it"
                  ? it
                  : ja;
    let message =
      dictionary[key] ??
      (this.locale === "es" ? key : (en[key] ?? key));
    if (values) {
      for (const [name, value] of Object.entries(values)) {
        message = message.replaceAll(`{${name}}`, String(value));
      }
    }
    return message;
  }

  setLocale(locale: Locale): void {
    this.locale = locale;
    if (typeof document !== "undefined") {
      document.documentElement.lang = locale;
    }
    try {
      window.localStorage.setItem(STORAGE_KEY, locale);
    } catch {
      // Persistence is best effort.
    }
  }
}

export const i18n = new I18n();
export const t = (key: string, values?: Record<string, string | number>): string =>
  i18n.t(key, values);
