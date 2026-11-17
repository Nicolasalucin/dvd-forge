import { en, es, type Dictionary } from "./messages";

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

/** A lazily loaded locale: UI strings plus per-code diagnostic meanings. */
interface LoadedLocale {
  messages: Dictionary;
  diagnostics: Record<string, string>;
}

/**
 * Spanish (source) and English (fallback) ship in the main bundle; the other
 * catalogs are separate chunks fetched when the language is selected, so the
 * app does not load ~400 KB of translations nobody reads.
 */
const LOADERS: Partial<Record<Locale, () => Promise<LoadedLocale>>> = {
  pt: () => import("./locales/pt").then((m) => ({ messages: m.pt, diagnostics: m.DIAGNOSTIC_PT })),
  fr: () => import("./locales/fr").then((m) => ({ messages: m.fr, diagnostics: m.DIAGNOSTIC_FR })),
  de: () => import("./locales/de").then((m) => ({ messages: m.de, diagnostics: m.DIAGNOSTIC_DE })),
  it: () => import("./locales/it").then((m) => ({ messages: m.it, diagnostics: m.DIAGNOSTIC_IT })),
  ja: () => import("./locales/ja").then((m) => ({ messages: m.ja, diagnostics: m.DIAGNOSTIC_JA })),
};

const STORAGE_KEY = "kinodisc_locale";
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
  /** Loaded lazy catalogs; reactive so the UI re-renders when one arrives. */
  loaded = $state<Partial<Record<Locale, LoadedLocale>>>({});

  constructor() {
    if (typeof document !== "undefined") {
      document.documentElement.lang = this.locale;
    }
    void this.ensure(this.locale);
  }

  /** Fetches a lazy catalog once. Until it arrives, English is shown. */
  async ensure(locale: Locale): Promise<void> {
    const loader = LOADERS[locale];
    if (!loader || this.loaded[locale]) return;
    try {
      const catalog = await loader();
      this.loaded = { ...this.loaded, [locale]: catalog };
    } catch {
      // Offline/broken chunk: English stays as the fallback.
    }
  }

  private dictionary(): Dictionary {
    if (this.locale === "es") return es;
    if (this.locale === "en") return en;
    return this.loaded[this.locale]?.messages ?? en;
  }

  /** True when the current locale has its own translation for `key`. */
  has(key: string): boolean {
    return this.locale === "es" || key in this.dictionary();
  }

  /** Per-code diagnostic meanings for the current locale, when loaded. */
  diagnosticMeanings(): Record<string, string> | null {
    return this.loaded[this.locale]?.diagnostics ?? null;
  }

  t(key: string, values?: Record<string, string | number>): string {
    let message =
      this.dictionary()[key] ?? (this.locale === "es" ? key : (en[key] ?? key));
    if (values) {
      for (const [name, value] of Object.entries(values)) {
        message = message.replaceAll(`{${name}}`, String(value));
      }
    }
    return message;
  }

  setLocale(locale: Locale): void {
    this.locale = locale;
    void this.ensure(locale);
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
