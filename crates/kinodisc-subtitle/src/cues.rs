use crate::style::{parse_color, SubtitleAlignment};
use crate::SubtitleError;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubtitleFormat {
    Srt,
    Ass,
    Vtt,
}

impl SubtitleFormat {
    pub fn from_path(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        match extension.as_str() {
            "srt" => Some(Self::Srt),
            "ass" | "ssa" => Some(Self::Ass),
            "vtt" => Some(Self::Vtt),
            _ => None,
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "srt" | "subrip" => Some(Self::Srt),
            "ass" | "ssa" => Some(Self::Ass),
            "vtt" | "webvtt" => Some(Self::Vtt),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct CueStyle {
    pub font_size_px: Option<f32>,
    pub fill_color: Option<[u8; 4]>,
    pub outline_color: Option<[u8; 4]>,
    pub alignment: Option<SubtitleAlignment>,
    pub bold: bool,
    pub italic: bool,
}

#[derive(Debug, Clone)]
pub struct SubtitleCue {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub style: CueStyle,
    pub forced: bool,
}

#[derive(Debug, Clone)]
pub struct SubtitleWarning {
    pub code: &'static str,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct ParsedSubtitles {
    pub format: SubtitleFormat,
    pub cues: Vec<SubtitleCue>,
    pub warnings: Vec<SubtitleWarning>,
}

pub fn parse_text(format: SubtitleFormat, text: &str) -> Result<ParsedSubtitles, SubtitleError> {
    let normalized = text
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    match format {
        SubtitleFormat::Srt => parse_srt(&normalized),
        SubtitleFormat::Ass => parse_ass(&normalized),
        SubtitleFormat::Vtt => parse_vtt(&normalized),
    }
}

fn parse_timestamp(value: &str, separator: char) -> Option<u64> {
    let value = value.trim();
    let mut parts = value.split(':').collect::<Vec<_>>();
    if parts.len() == 2 {
        parts.insert(0, "0");
    }
    if parts.len() != 3 {
        return None;
    }
    let hours = parts[0].parse::<u64>().ok()?;
    let minutes = parts[1].parse::<u64>().ok()?;
    let seconds_part = parts[2].replace(separator, ".");
    let seconds = seconds_part.parse::<f64>().ok()?;
    Some(hours * 3_600_000 + minutes * 60_000 + (seconds * 1000.0).round() as u64)
}

fn parse_range(line: &str) -> Option<(u64, u64)> {
    let (start, rest) = line.split_once("-->")?;
    let end = rest.split_whitespace().next()?;
    Some((parse_timestamp(start, ',')?, parse_timestamp(end, ',')?))
}

fn clean_text(value: &str) -> (String, CueStyle, bool) {
    let mut style = CueStyle::default();
    let mut forced = false;
    let mut text = value.replace("\\N", "\n").replace("\\n", "\n");
    if let Some(position) = text.find("{\\an") {
        if let Some(end) = text[position..].find('}') {
            let tag = &text[position + 3..position + end];
            style.alignment = match tag.chars().next() {
                Some('1') => Some(SubtitleAlignment::BottomLeft),
                Some('2') => Some(SubtitleAlignment::BottomCenter),
                Some('3') => Some(SubtitleAlignment::BottomRight),
                Some('4') => Some(SubtitleAlignment::Center),
                Some('5') => Some(SubtitleAlignment::TopCenter),
                _ => None,
            };
            text.replace_range(position..position + end + 1, "");
        }
    }
    let mut output = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '<' => {
                let mut tag = String::new();
                for next in chars.by_ref() {
                    if next == '>' {
                        break;
                    }
                    tag.push(next);
                }
                if tag.eq_ignore_ascii_case("i") {
                    style.italic = true;
                } else if tag.eq_ignore_ascii_case("b") {
                    style.bold = true;
                } else if tag.eq_ignore_ascii_case("u") {
                    // underline is not representable in the current style model
                }
            }
            '{' => {
                for next in chars.by_ref() {
                    if next == '}' {
                        break;
                    }
                }
            }
            _ => output.push(ch),
        }
    }
    if output.to_ascii_lowercase().contains("forced") {
        forced = true;
    }
    (output.trim().to_string(), style, forced)
}

fn parse_srt(text: &str) -> Result<ParsedSubtitles, SubtitleError> {
    let mut cues = Vec::new();
    let mut warnings = Vec::new();
    for block in text.split("\n\n") {
        let block = block.trim();
        if block.is_empty() {
            continue;
        }
        let lines = block.lines().collect::<Vec<_>>();
        let range = lines.iter().position(|line| line.contains("-->"));
        let Some(range_index) = range else {
            warnings.push(SubtitleWarning {
                code: "W_SUBTITLE_SOURCE_FORMAT",
                message: "Bloque SRT sin rango de tiempos; se omite.".to_string(),
            });
            continue;
        };
        let Some((start_ms, end_ms)) = parse_range(lines[range_index]) else {
            return Err(SubtitleError::Invalid("rango SRT inválido".to_string()));
        };
        let (text, style, forced) = clean_text(&lines[range_index + 1..].join("\n"));
        if text.is_empty() || end_ms <= start_ms {
            warnings.push(SubtitleWarning {
                code: "W_SUBTITLE_DROP",
                message: "Cue SRT vacío o con tiempos no positivos; se omite.".to_string(),
            });
            continue;
        }
        cues.push(SubtitleCue {
            start_ms,
            end_ms,
            text,
            style,
            forced,
        });
    }
    finish(SubtitleFormat::Srt, cues, warnings)
}

fn parse_vtt(text: &str) -> Result<ParsedSubtitles, SubtitleError> {
    let body = text
        .split_once("\n\n")
        .filter(|(header, _)| header.trim_start().starts_with("WEBVTT"))
        .map(|(_, body)| body)
        .unwrap_or(text);
    let mut cues = Vec::new();
    let mut warnings = Vec::new();
    for block in body.split("\n\n") {
        let block = block.trim();
        if block.is_empty() || block.starts_with("NOTE") {
            continue;
        }
        let lines = block.lines().collect::<Vec<_>>();
        let Some(range_index) = lines.iter().position(|line| line.contains("-->")) else {
            continue;
        };
        let line = lines[range_index].replace('.', ",");
        let Some((start_ms, end_ms)) = parse_range(&line) else {
            return Err(SubtitleError::Invalid("rango VTT inválido".to_string()));
        };
        let mut text = lines[range_index + 1..].join("\n");
        if let Some((_, settings)) = lines[range_index].split_once("-->") {
            if let Some(setting) = settings
                .split_whitespace()
                .find(|part| part.starts_with("align:"))
            {
                let alignment = setting.trim_start_matches("align:");
                let _ = alignment;
            }
        }
        text = text
            .replace("<v ", "")
            .replace("</v>", "")
            .replace("<c>", "")
            .replace("</c>", "");
        let (text, style, forced) = clean_text(&text);
        if text.is_empty() || end_ms <= start_ms {
            warnings.push(SubtitleWarning {
                code: "W_SUBTITLE_DROP",
                message: "Cue VTT vacío o con tiempos no positivos; se omite.".to_string(),
            });
            continue;
        }
        cues.push(SubtitleCue {
            start_ms,
            end_ms,
            text,
            style,
            forced,
        });
    }
    finish(SubtitleFormat::Vtt, cues, warnings)
}

#[derive(Clone, Default)]
struct AssStyle {
    font_size: Option<f32>,
    fill_color: Option<[u8; 4]>,
    outline_color: Option<[u8; 4]>,
    alignment: Option<SubtitleAlignment>,
    bold: bool,
    italic: bool,
}

fn parse_ass_alignment(value: &str) -> Option<SubtitleAlignment> {
    match value.trim().parse::<u8>().ok()? {
        1..=3 => Some(SubtitleAlignment::BottomLeft),
        4..=6 => Some(SubtitleAlignment::BottomCenter),
        7..=9 => Some(SubtitleAlignment::BottomRight),
        10 => Some(SubtitleAlignment::Center),
        _ => None,
    }
}

fn parse_ass(text: &str) -> Result<ParsedSubtitles, SubtitleError> {
    let mut styles: HashMap<String, AssStyle> = HashMap::new();
    let mut cues = Vec::new();
    let mut warnings = Vec::new();
    let mut section = "";
    let mut style_format: Vec<String> = Vec::new();
    let mut event_format: Vec<String> = Vec::new();
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') {
            section = line;
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if section.eq_ignore_ascii_case("[V4+ Styles]")
            || section.eq_ignore_ascii_case("[V4 Styles]")
        {
            if key.eq_ignore_ascii_case("Format") {
                style_format = value
                    .split(',')
                    .map(|part| part.trim().to_string())
                    .collect();
            } else if key.eq_ignore_ascii_case("Style") {
                let fields = value
                    .splitn(style_format.len().max(1), ',')
                    .collect::<Vec<_>>();
                let get = |name: &str| {
                    style_format
                        .iter()
                        .position(|item| item.eq_ignore_ascii_case(name))
                        .and_then(|index| fields.get(index))
                        .map(|value| value.trim())
                };
                let name = fields
                    .first()
                    .copied()
                    .unwrap_or("Default")
                    .trim()
                    .to_string();
                styles.insert(
                    name,
                    AssStyle {
                        font_size: get("Fontsize").and_then(|value| value.parse().ok()),
                        fill_color: get("PrimaryColour").and_then(parse_color),
                        outline_color: get("OutlineColour").and_then(parse_color),
                        alignment: get("Alignment").and_then(parse_ass_alignment),
                        bold: get("Bold").is_some_and(|value| value != "0"),
                        italic: get("Italic").is_some_and(|value| value != "0"),
                    },
                );
            }
        } else if section.eq_ignore_ascii_case("[Events]") {
            if key.eq_ignore_ascii_case("Format") {
                event_format = value
                    .split(',')
                    .map(|part| part.trim().to_string())
                    .collect();
            } else if key.eq_ignore_ascii_case("Dialogue") {
                let fields = value
                    .splitn(event_format.len().max(1), ',')
                    .collect::<Vec<_>>();
                let get = |name: &str| {
                    event_format
                        .iter()
                        .position(|item| item.eq_ignore_ascii_case(name))
                        .and_then(|index| fields.get(index))
                        .map(|value| value.trim())
                };
                let start = get("Start").and_then(|value| parse_timestamp(value, '.'));
                let end = get("End").and_then(|value| parse_timestamp(value, '.'));
                let (Some(start_ms), Some(end_ms)) = (start, end) else {
                    warnings.push(SubtitleWarning {
                        code: "W_SUBTITLE_SOURCE_FORMAT",
                        message: "Diálogo ASS con tiempos inválidos; se omite.".to_string(),
                    });
                    continue;
                };
                let style_name = get("Style").unwrap_or("Default");
                let ass_style = styles.get(style_name).cloned().unwrap_or_default();
                let (text, style, forced) = clean_text(get("Text").unwrap_or(""));
                if text.is_empty() || end_ms <= start_ms {
                    warnings.push(SubtitleWarning {
                        code: "W_SUBTITLE_DROP",
                        message: "Diálogo ASS vacío o con tiempos no positivos; se omite."
                            .to_string(),
                    });
                    continue;
                }
                cues.push(SubtitleCue {
                    start_ms,
                    end_ms,
                    text,
                    style: CueStyle {
                        font_size_px: ass_style.font_size,
                        fill_color: ass_style.fill_color,
                        outline_color: ass_style.outline_color,
                        alignment: style.alignment.or(ass_style.alignment),
                        bold: ass_style.bold || style.bold,
                        italic: ass_style.italic || style.italic,
                    },
                    forced,
                });
            }
        }
    }
    finish(SubtitleFormat::Ass, cues, warnings)
}

fn finish(
    format: SubtitleFormat,
    mut cues: Vec<SubtitleCue>,
    mut warnings: Vec<SubtitleWarning>,
) -> Result<ParsedSubtitles, SubtitleError> {
    if cues.is_empty() {
        return Err(SubtitleError::Invalid(
            "el subtítulo no contiene cues utilizables".to_string(),
        ));
    }
    cues.sort_by_key(|cue| (cue.start_ms, cue.end_ms));
    for pair in cues.windows(2) {
        if pair[1].start_ms < pair[0].end_ms {
            warnings.push(SubtitleWarning {
                code: "W_SUBTITLE_OVERFLOW",
                message: "Hay cues solapados; se serializarán en orden determinista.".to_string(),
            });
            break;
        }
    }
    Ok(ParsedSubtitles {
        format,
        cues,
        warnings,
    })
}
