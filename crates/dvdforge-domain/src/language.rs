//! ISO 639 catalog shared by DVD (2-letter) and Blu-ray/AVCHD (3-letter).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum LanguageCode {
    Spanish,
    English,
    Japanese,
    Portuguese,
    French,
    German,
    Italian,
    Russian,
    Chinese,
    Korean,
    Catalan,
    Basque,
    Galician,
    Arabic,
    Hindi,
    Undetermined,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LanguageInfo {
    pub code: String,
    pub iso_639_1: String,
    pub iso_639_2: String,
    pub iso_639_2_bibliographic: String,
    pub name_es: String,
    pub name_en: String,
}

impl LanguageCode {
    pub const ALL: [Self; 16] = [
        Self::Spanish,
        Self::English,
        Self::Japanese,
        Self::Portuguese,
        Self::French,
        Self::German,
        Self::Italian,
        Self::Russian,
        Self::Chinese,
        Self::Korean,
        Self::Catalan,
        Self::Basque,
        Self::Galician,
        Self::Arabic,
        Self::Hindi,
        Self::Undetermined,
    ];

    /// Two-letter code used by DVD-Video (`lang` in dvdauthor).
    /// `und` is intentionally kept for undetermined/original audio.
    pub const fn iso_639_1(self) -> &'static str {
        match self {
            Self::Spanish => "es",
            Self::English => "en",
            Self::Japanese => "ja",
            Self::Portuguese => "pt",
            Self::French => "fr",
            Self::German => "de",
            Self::Italian => "it",
            Self::Russian => "ru",
            Self::Chinese => "zh",
            Self::Korean => "ko",
            Self::Catalan => "ca",
            Self::Basque => "eu",
            Self::Galician => "gl",
            Self::Arabic => "ar",
            Self::Hindi => "hi",
            Self::Undetermined => "und",
        }
    }

    /// Preferred ISO 639-2/T code used by Blu-ray/AVCHD.
    pub const fn iso_639_2(self) -> &'static str {
        match self {
            Self::Spanish => "spa",
            Self::English => "eng",
            Self::Japanese => "jpn",
            Self::Portuguese => "por",
            Self::French => "fra",
            Self::German => "deu",
            Self::Italian => "ita",
            Self::Russian => "rus",
            Self::Chinese => "zho",
            Self::Korean => "kor",
            Self::Catalan => "cat",
            Self::Basque => "eus",
            Self::Galician => "glg",
            Self::Arabic => "ara",
            Self::Hindi => "hin",
            Self::Undetermined => "und",
        }
    }

    /// ISO 639-2/B bibliographic alias, also accepted on import.
    pub const fn iso_639_2_bibliographic(self) -> &'static str {
        match self {
            Self::French => "fre",
            Self::German => "ger",
            Self::Chinese => "chi",
            Self::Basque => "baq",
            _ => self.iso_639_2(),
        }
    }

    pub const fn name_spanish(self) -> &'static str {
        match self {
            Self::Spanish => "Español",
            Self::English => "Inglés",
            Self::Japanese => "Japonés",
            Self::Portuguese => "Portugués",
            Self::French => "Francés",
            Self::German => "Alemán",
            Self::Italian => "Italiano",
            Self::Russian => "Ruso",
            Self::Chinese => "Chino",
            Self::Korean => "Coreano",
            Self::Catalan => "Catalán",
            Self::Basque => "Euskera",
            Self::Galician => "Gallego",
            Self::Arabic => "Árabe",
            Self::Hindi => "Hindi",
            Self::Undetermined => "Indeterminado / Original",
        }
    }

    pub const fn name_english(self) -> &'static str {
        match self {
            Self::Spanish => "Spanish",
            Self::English => "English",
            Self::Japanese => "Japanese",
            Self::Portuguese => "Portuguese",
            Self::French => "French",
            Self::German => "German",
            Self::Italian => "Italian",
            Self::Russian => "Russian",
            Self::Chinese => "Chinese",
            Self::Korean => "Korean",
            Self::Catalan => "Catalan",
            Self::Basque => "Basque",
            Self::Galician => "Galician",
            Self::Arabic => "Arabic",
            Self::Hindi => "Hindi",
            Self::Undetermined => "Undetermined / Original",
        }
    }

    /// Parses 2-letter ISO 639-1, ISO 639-2/T and ISO 639-2/B aliases.
    pub fn from_code(code: &str) -> Option<Self> {
        let normalized = code.trim().to_ascii_lowercase();
        Some(match normalized.as_str() {
            "es" | "spa" => Self::Spanish,
            "en" | "eng" => Self::English,
            "ja" | "jpn" => Self::Japanese,
            "pt" | "por" => Self::Portuguese,
            "fr" | "fra" | "fre" => Self::French,
            "de" | "deu" | "ger" => Self::German,
            "it" | "ita" => Self::Italian,
            "ru" | "rus" => Self::Russian,
            "zh" | "zho" | "chi" => Self::Chinese,
            "ko" | "kor" => Self::Korean,
            "ca" | "cat" => Self::Catalan,
            "eu" | "eus" | "baq" => Self::Basque,
            "gl" | "glg" => Self::Galician,
            "ar" | "ara" => Self::Arabic,
            "hi" | "hin" => Self::Hindi,
            "und" | "" => Self::Undetermined,
            _ => return None,
        })
    }

    /// Reads the `tags.language` value returned by FFprobe and falls back to
    /// undetermined when the tag is absent or unknown.
    pub fn from_ffprobe_tag(tag: Option<&str>) -> Self {
        tag.and_then(Self::from_code).unwrap_or(Self::Undetermined)
    }

    pub fn info(self) -> LanguageInfo {
        LanguageInfo {
            code: self.iso_639_1().to_string(),
            iso_639_1: self.iso_639_1().to_string(),
            iso_639_2: self.iso_639_2().to_string(),
            iso_639_2_bibliographic: self.iso_639_2_bibliographic().to_string(),
            name_es: self.name_spanish().to_string(),
            name_en: self.name_english().to_string(),
        }
    }

    pub fn catalog() -> Vec<LanguageInfo> {
        Self::ALL.into_iter().map(Self::info).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogs_the_official_dvd_and_bluray_codes() {
        assert_eq!(LanguageCode::Spanish.iso_639_1(), "es");
        assert_eq!(LanguageCode::Spanish.iso_639_2(), "spa");
        assert_eq!(LanguageCode::French.iso_639_2(), "fra");
        assert_eq!(LanguageCode::French.iso_639_2_bibliographic(), "fre");
        assert_eq!(LanguageCode::German.iso_639_2(), "deu");
        assert_eq!(LanguageCode::German.iso_639_2_bibliographic(), "ger");
        assert_eq!(LanguageCode::Chinese.iso_639_2(), "zho");
        assert_eq!(LanguageCode::Chinese.iso_639_2_bibliographic(), "chi");
        assert_eq!(LanguageCode::Basque.iso_639_2(), "eus");
        assert_eq!(LanguageCode::Basque.iso_639_2_bibliographic(), "baq");
        assert_eq!(LanguageCode::Undetermined.iso_639_1(), "und");
        assert_eq!(LanguageCode::Undetermined.iso_639_2(), "und");
    }

    #[test]
    fn parses_iso_639_1_and_both_iso_639_2_forms() {
        for language in LanguageCode::ALL {
            assert_eq!(
                LanguageCode::from_code(language.iso_639_1()),
                Some(language)
            );
            assert_eq!(
                LanguageCode::from_code(language.iso_639_2()),
                Some(language)
            );
            assert_eq!(
                LanguageCode::from_code(language.iso_639_2_bibliographic()),
                Some(language)
            );
        }
        assert_eq!(LanguageCode::from_code("FRE"), Some(LanguageCode::French));
        assert_eq!(LanguageCode::from_code(" ger "), Some(LanguageCode::German));
        assert_eq!(LanguageCode::from_code("xyz"), None);
    }

    #[test]
    fn ffprobe_tags_are_mapped_to_the_catalog() {
        assert_eq!(
            LanguageCode::from_ffprobe_tag(Some("spa")),
            LanguageCode::Spanish
        );
        assert_eq!(
            LanguageCode::from_ffprobe_tag(Some("jpn")),
            LanguageCode::Japanese
        );
        assert_eq!(
            LanguageCode::from_ffprobe_tag(Some("fre")),
            LanguageCode::French
        );
        assert_eq!(
            LanguageCode::from_ffprobe_tag(Some("EU")),
            LanguageCode::Basque
        );
        assert_eq!(
            LanguageCode::from_ffprobe_tag(None),
            LanguageCode::Undetermined
        );
        assert_eq!(
            LanguageCode::from_ffprobe_tag(Some("unknown-language")),
            LanguageCode::Undetermined
        );
    }

    #[test]
    fn catalog_exposes_all_requested_languages() {
        let catalog = LanguageCode::catalog();
        assert_eq!(catalog.len(), 16);
        assert!(catalog.iter().any(|item| item.iso_639_1 == "ja"));
        assert!(catalog.iter().any(|item| item.iso_639_2 == "spa"));
        assert!(catalog
            .iter()
            .any(|item| item.iso_639_2_bibliographic == "fre"));
    }
}
