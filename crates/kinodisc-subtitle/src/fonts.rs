use crate::style::SubtitleStyle;
use crate::SubtitleError;
use ab_glyph::FontArc;
use std::path::Path;
use std::sync::Arc;

#[derive(Clone)]
pub struct ResolvedFont {
    pub font: FontArc,
    pub family: String,
    pub digest: String,
}

pub struct FontProvider {
    db: fontdb::Database,
}

impl Default for FontProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl FontProvider {
    pub fn new() -> Self {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        Self { db }
    }

    /// A system font by family and weight, with the same sans-serif
    /// fallbacks as subtitles (menu labels drawn by the CLI compositor).
    pub fn resolve_family(&self, family: &str, weight: u16) -> Option<FontArc> {
        let family = family.trim().trim_matches('"');
        let families = [
            family,
            "Segoe UI",
            "DejaVu Sans",
            "Arial",
            "Helvetica",
            "Liberation Sans",
            "Noto Sans",
        ];
        for name in families.into_iter().filter(|name| !name.is_empty()) {
            let query = fontdb::Query {
                families: &[fontdb::Family::Name(name)],
                weight: fontdb::Weight(weight),
                stretch: fontdb::Stretch::Normal,
                style: fontdb::Style::Normal,
            };
            let Some(id) = self.db.query(&query) else {
                continue;
            };
            let mut font = None;
            self.db.with_face_data(id, |data, face_index| {
                font = ab_glyph::FontVec::try_from_vec_and_index(data.to_vec(), face_index).ok();
            });
            if let Some(font) = font {
                return Some(FontArc::new(font));
            }
        }
        None
    }

    /// Only the named family (no fallback): emoji/symbol fallback fonts.
    pub fn resolve_exact(&self, family: &str, weight: u16) -> Option<FontArc> {
        let query = fontdb::Query {
            families: &[fontdb::Family::Name(family)],
            weight: fontdb::Weight(weight),
            stretch: fontdb::Stretch::Normal,
            style: fontdb::Style::Normal,
        };
        let id = self.db.query(&query)?;
        let mut font = None;
        self.db.with_face_data(id, |data, face_index| {
            font = ab_glyph::FontVec::try_from_vec_and_index(data.to_vec(), face_index).ok();
        });
        font.map(FontArc::new)
    }

    pub fn resolve(&self, style: &SubtitleStyle) -> Result<ResolvedFont, SubtitleError> {
        if let Some(path) = &style.font_path {
            let bytes = std::fs::read(path)?;
            let font = FontArc::try_from_vec(bytes)
                .map_err(|error| SubtitleError::Font(error.to_string()))?;
            return Ok(ResolvedFont {
                font,
                family: style.font_family.clone(),
                digest: digest_path(path)?,
            });
        }
        let families = [
            style.font_family.as_str(),
            "DejaVu Sans",
            "Arial",
            "Helvetica",
            "Liberation Sans",
            "Noto Sans",
            "sans-serif",
        ];
        for family in families {
            let query = fontdb::Query {
                families: &[fontdb::Family::Name(family)],
                weight: fontdb::Weight(style.font_weight),
                stretch: fontdb::Stretch::Normal,
                style: if style.italic {
                    fontdb::Style::Italic
                } else {
                    fontdb::Style::Normal
                },
            };
            let Some(id) = self.db.query(&query) else {
                continue;
            };
            let mut bytes = None;
            self.db.with_face_data(id, |data, _face_index| {
                bytes = Some(data.to_vec());
            });
            let Some(bytes) = bytes else { continue };
            let digest = digest_bytes(&bytes);
            let font = FontArc::try_from_vec(bytes)
                .map_err(|error| SubtitleError::Font(error.to_string()))?;
            return Ok(ResolvedFont {
                font,
                family: family.to_string(),
                digest,
            });
        }
        Err(SubtitleError::Font(format!(
            "no se encontró una fuente para «{}»",
            style.font_family
        )))
    }
}

fn digest_path(path: &Path) -> Result<String, SubtitleError> {
    let bytes = std::fs::read(path)?;
    Ok(digest_bytes(&bytes))
}

fn digest_bytes(bytes: &[u8]) -> String {
    // Small, deterministic FNV-1a used only for cache identity.
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

impl std::fmt::Debug for ResolvedFont {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResolvedFont")
            .field("family", &self.family)
            .field("digest", &self.digest)
            .finish()
    }
}

/// Unused but keeps the Arc import meaningful for future shared caches.
#[allow(dead_code)]
fn shared(font: FontArc) -> Arc<FontArc> {
    Arc::new(font)
}
