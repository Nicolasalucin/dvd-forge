export interface LanguageEntry {
  code: string;
  iso2: string;
  iso3: string;
  iso3Bibliographic: string;
  nameEs: string;
  nameEn: string;
  namePt: string;
  nameFr: string;
  nameDe: string;
  nameIt: string;
  nameJa: string;
}

export const LANGUAGES: LanguageEntry[] = [
  { code: "es", iso2: "es", iso3: "spa", iso3Bibliographic: "spa", nameEs: "Español", nameEn: "Spanish", namePt: "Espanhol", nameFr: "Espagnol", nameDe: "Spanisch", nameIt: "Spagnolo", nameJa: "スペイン語" },
  { code: "en", iso2: "en", iso3: "eng", iso3Bibliographic: "eng", nameEs: "Inglés", nameEn: "English", namePt: "Inglês", nameFr: "Anglais", nameDe: "Englisch", nameIt: "Inglese", nameJa: "英語" },
  { code: "ja", iso2: "ja", iso3: "jpn", iso3Bibliographic: "jpn", nameEs: "Japonés", nameEn: "Japanese", namePt: "Japonês", nameFr: "Japonais", nameDe: "Japanisch", nameIt: "Giapponese", nameJa: "日本語" },
  { code: "pt", iso2: "pt", iso3: "por", iso3Bibliographic: "por", nameEs: "Portugués", nameEn: "Portuguese", namePt: "Português", nameFr: "Portugais", nameDe: "Portugiesisch", nameIt: "Portoghese", nameJa: "ポルトガル語" },
  { code: "fr", iso2: "fr", iso3: "fra", iso3Bibliographic: "fre", nameEs: "Francés", nameEn: "French", namePt: "Francês", nameFr: "Français", nameDe: "Französisch", nameIt: "Francese", nameJa: "フランス語" },
  { code: "de", iso2: "de", iso3: "deu", iso3Bibliographic: "ger", nameEs: "Alemán", nameEn: "German", namePt: "Alemão", nameFr: "Allemand", nameDe: "Deutsch", nameIt: "Tedesco", nameJa: "ドイツ語" },
  { code: "it", iso2: "it", iso3: "ita", iso3Bibliographic: "ita", nameEs: "Italiano", nameEn: "Italian", namePt: "Italiano", nameFr: "Italien", nameDe: "Italienisch", nameIt: "Italiano", nameJa: "イタリア語" },
  { code: "ru", iso2: "ru", iso3: "rus", iso3Bibliographic: "rus", nameEs: "Ruso", nameEn: "Russian", namePt: "Russo", nameFr: "Russe", nameDe: "Russisch", nameIt: "Russo", nameJa: "ロシア語" },
  { code: "zh", iso2: "zh", iso3: "zho", iso3Bibliographic: "chi", nameEs: "Chino", nameEn: "Chinese", namePt: "Chinês", nameFr: "Chinois", nameDe: "Chinesisch", nameIt: "Cinese", nameJa: "中国語" },
  { code: "ko", iso2: "ko", iso3: "kor", iso3Bibliographic: "kor", nameEs: "Coreano", nameEn: "Korean", namePt: "Coreano", nameFr: "Coréen", nameDe: "Koreanisch", nameIt: "Coreano", nameJa: "韓国語" },
  { code: "ca", iso2: "ca", iso3: "cat", iso3Bibliographic: "cat", nameEs: "Catalán", nameEn: "Catalan", namePt: "Catalão", nameFr: "Catalan", nameDe: "Katalanisch", nameIt: "Catalano", nameJa: "カタルーニャ語" },
  { code: "eu", iso2: "eu", iso3: "eus", iso3Bibliographic: "baq", nameEs: "Euskera", nameEn: "Basque", namePt: "Basco", nameFr: "Basque", nameDe: "Baskisch", nameIt: "Basco", nameJa: "バスク語" },
  { code: "gl", iso2: "gl", iso3: "glg", iso3Bibliographic: "glg", nameEs: "Gallego", nameEn: "Galician", namePt: "Galego", nameFr: "Galicien", nameDe: "Galicisch", nameIt: "Galiziano", nameJa: "ガリシア語" },
  { code: "ar", iso2: "ar", iso3: "ara", iso3Bibliographic: "ara", nameEs: "Árabe", nameEn: "Arabic", namePt: "Árabe", nameFr: "Arabe", nameDe: "Arabisch", nameIt: "Arabo", nameJa: "アラビア語" },
  { code: "hi", iso2: "hi", iso3: "hin", iso3Bibliographic: "hin", nameEs: "Hindi", nameEn: "Hindi", namePt: "Hindi", nameFr: "Hindi", nameDe: "Hindi", nameIt: "Hindi", nameJa: "ヒンディー語" },
  { code: "und", iso2: "und", iso3: "und", iso3Bibliographic: "und", nameEs: "Indeterminado / Original", nameEn: "Undetermined / Original", namePt: "Indeterminado / Original", nameFr: "Indéterminé / Original", nameDe: "Unbestimmt / Original", nameIt: "Indeterminato / Originale", nameJa: "未指定 / オリジナル" },
];

export function normalizeLanguageCode(value: string | null | undefined): string {
  const normalized = (value ?? "").trim().toLowerCase();
  if (!normalized) return "und";
  return LANGUAGES.find(
    (language) =>
      language.code === normalized ||
      language.iso2 === normalized ||
      language.iso3 === normalized ||
      language.iso3Bibliographic === normalized,
  )?.code ?? "und";
}

export function languageEntry(code: string): LanguageEntry {
  const normalized = normalizeLanguageCode(code);
  return LANGUAGES.find((language) => language.code === normalized) ?? LANGUAGES[LANGUAGES.length - 1];
}

export function languageLabel(
  code: string,
  locale: "es" | "en" | "pt" | "fr" | "de" | "it" | "ja",
): string {
  const entry = languageEntry(code);
  const name =
    locale === "es"
      ? entry.nameEs
      : locale === "pt"
        ? entry.namePt
        : locale === "fr"
          ? entry.nameFr
          : locale === "de"
            ? entry.nameDe
            : locale === "it"
              ? entry.nameIt
              : locale === "ja"
                ? entry.nameJa
                : entry.nameEn;
  const aliases =
    entry.iso3Bibliographic !== entry.iso3
      ? `${entry.iso2} / ${entry.iso3} / ${entry.iso3Bibliographic}`
      : `${entry.iso2} / ${entry.iso3}`;
  return `${name} (${aliases})`;
}
