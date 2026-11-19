import { formatDecimal, i18n, t } from "./index.svelte";

/** "2.0" from the engine, written the interface's way ("2,0" in Spanish). */
function fillNumber(value: string): string {
  const number = Number(value.replace(",", "."));
  return Number.isFinite(number) ? formatDecimal(number, 1) : value;
}
import { DIAGNOSTIC_EN } from "./diagnosticCodes";
import { formatBytes } from "../utils/format";
export interface DiagnosticLike {
  code: string;
  message: string;
}

type Values = Record<string, string | number>;

interface PatternRule {
  key: string;
  pattern: RegExp;
  values: (match: RegExpMatchArray) => Values;
}

/**
 * The planner emits stable diagnostic codes with Spanish messages. The UI
 * prefers the code so the text can follow the selected locale; the message is
 * only parsed for the values that were interpolated by Rust.
 */
const PATTERNS: PatternRule[] = [
  {
    key: "Ningún botón de los menús abre «{title}»: no se podrá elegir desde el disco.",
    pattern: /Ningún botón de los menús abre «(.*)»: /,
    values: (match) => ({ title: match[1] }),
  },
  {
    key: "El episodio {episode} tiene {count} pistas de audio y el primero tiene {first}. dvdauthor usa el layout del primer título.",
    pattern: /El episodio (\d+) tiene (\d+) pistas de audio y el primero tiene (\d+)\./,
    values: (match) => ({ episode: match[1], count: match[2], first: match[3] }),
  },
  {
    key: "El episodio {episode} referencia audio externo; el encoder usa el stream embebido del vídeo.",
    pattern: /El episodio (\d+) referencia audio externo/,
    values: (match) => ({ episode: match[1] }),
  },
  {
    key: "El episodio {episode} tiene subtítulos internos; se extraerán del vídeo con FFmpeg antes de multiplexarlos.",
    pattern: /El episodio (\d+) tiene subtítulos internos/,
    values: (match) => ({ episode: match[1] }),
  },
  {
    key: "Se activó Half-D1 automáticamente: duración {duration} ms y bitrate calculado {bitrate} kbps.",
    pattern: /duración (\d+) ms y bitrate calculado (\d+) kbps/,
    values: (match) => ({ duration: match[1], bitrate: match[2] }),
  },
  {
    key: "El bitrate calculado {bitrate} kbps supera el máximo {max} kbps; el disco quedará por debajo del objetivo de llenado.",
    pattern: /El bitrate calculado (\d+) kbps supera el máximo (\d+) kbps;/,
    values: (match) => ({ bitrate: match[1], max: match[2] }),
  },
  {
    key: "D1 está forzado pero el bitrate calculado ({bitrate} kbps) es inferior a {min} kbps.",
    pattern: /D1 está forzado pero el bitrate calculado \((\d+) kbps\) es inferior a (\d+) kbps/,
    values: (match) => ({ bitrate: match[1], min: match[2] }),
  },
  {
    key: "El episodio {episode} se limita a {bitrate} kbps para respetar el máximo total de DVD con sus pistas de audio.",
    pattern: /El episodio (\d+) se limita a (\d+) kbps/,
    values: (match) => ({ episode: match[1], bitrate: match[2] }),
  },
  {
    key: "El plan solo llena el {percent}% de la capacidad efectiva; el contenido es demasiado corto para alcanzar el 99% sin superar el bitrate máximo.",
    pattern: /El plan solo llena el ([\d.,]+)% de la capacidad efectiva/,
    values: (match) => ({ percent: fillNumber(match[1]) }),
  },
  {
    key: "El episodio {episode} tiene varias pistas de audio; AVCHD usará la primera.",
    pattern: /El episodio (\d+) tiene varias pistas de audio/,
    values: (match) => ({ episode: match[1] }),
  },
  {
    key: "El bitrate calculado {bitrate} kbps supera el máximo de {max} kbps para {capacity}; el disco quedará por debajo del objetivo de llenado.",
    pattern: /El bitrate calculado (\d+) kbps supera el máximo de (\d+) kbps para (.+?);/,
    values: (match) => ({ bitrate: match[1], max: match[2], capacity: match[3] }),
  },
  {
    key: "El plan llena el {percent}% de la capacidad efectiva de {capacity}; el contenido es demasiado corto para aprovecharla.",
    pattern: /El plan llena el ([\d.,]+)% de la capacidad efectiva de (.+?);/,
    values: (match) => ({ percent: fillNumber(match[1]), capacity: match[2] }),
  },
];

const FIXED: Record<string, string> = {
  E_PLAN_INFEASIBLE: "El plan no es viable con el perfil y presupuesto actuales.",
  E_AVCHD_PLAN_INFEASIBLE: "El plan AVCHD supera el presupuesto del disco.",
  W_AVCHD_SUBTITLES_IGNORED:
    "Los subtítulos externos de AVCHD/Blu-ray se conservan en el proyecto, pero todavía no se multiplexan en la imagen.",
  W_AVCHD_SD_RECOMMENDED:
    "Más de 5 horas de contenido: considera resolución SD para conservar calidad y capacidad.",
  W_INTRO_ASSET_MISSING:
    "El First Play referencia un asset inexistente; no se reserva espacio para la intro.",
  W_INTRO_DURATION:
    "No se pudo determinar la duración de la intro; no se reserva espacio.",
};

export function localizeDiagnostic(diagnostic: DiagnosticLike): string {
  // Prefer a translation that keeps the interpolated values, but only when the
  // current locale really has it: `t()` silently falls back to English.
  const rule = PATTERNS.find((candidate) =>
    candidate.pattern.test(diagnostic.message),
  );
  if (rule && i18n.has(rule.key)) {
    const match = diagnostic.message.match(rule.pattern);
    if (match) return t(rule.key, rule.values(match));
  }
  const fixed = FIXED[diagnostic.code];
  if (fixed && i18n.has(fixed)) return t(fixed);
  if (i18n.has(diagnostic.message)) return t(diagnostic.message);
  // Per-code meaning in the current locale, so no locale shows Spanish text.
  const table = i18n.locale === "en" ? DIAGNOSTIC_EN : i18n.diagnosticMeanings();
  const generic = table?.[diagnostic.code];
  if (generic) return generic;
  if (rule) {
    const match = diagnostic.message.match(rule.pattern);
    if (match) return t(rule.key, rule.values(match));
  }
  if (fixed) return t(fixed);
  const english = DIAGNOSTIC_EN[diagnostic.code];
  return english ?? t(diagnostic.message);
}

/** Localizes any backend text that may carry a diagnostic code. */
export function localizeMessage(code: string | null | undefined, message: string): string {
  return code ? localizeDiagnostic({ code, message }) : t(message);
}

const JOB_STAGES = new Set([
  "Sondeando medios",
  "Calculando bitrate",
  "Codificando intro",
  "Codificando episodios",
  "Codificando episodios y compilando menús",
  "Compilando menús con spumux",
  "Compilando DVD con dvdauthor",
  "Construyendo ISO",
  "Calculando bitrate AVCHD",
  "Codificando episodios AVCHD",
  "Multiplexando AVCHD con tsMuxeR",
  "Multiplexando AVCHD",
  "Codificando episodios HD",
  "Multiplexando el disco HD",
  "Calculando bitrate HD",
  "Validando ISO HD",
  "Multiplexando BDMV con menús HDMV IG",
  "Validando ISO AVCHD",
]);

export function localizeJobStage(label: string): string {
  return JOB_STAGES.has(label) ? t(label) : label;
}

/**
 * Localizes a backend error string of the form `E_CODE: detalle`. Other
 * locales show the per-code meaning followed by the technical detail.
 */
export function localizeError(message: string): string {
  const match = /^([EW]_[A-Z0-9_]+): ([\s\S]*)$/.exec(message.trim());
  if (!match) return message;
  const localized = localizeDiagnostic({ code: match[1], message: match[2] });
  // The original is only worth appending when it carries details the
  // translation lacks (paths, tool output), not when it is a known sentence.
  if (localized === match[2] || i18n.has(match[2])) return localized;
  return `${localized} (${match[2]})`;
}

/**
 * Templates of the engine's log lines (Rust `format!` text) and the key they
 * translate to; `{…}` holes match, in order, the `{name}` placeholders of the key.
 */
const LOG_TEMPLATES: Array<[string, string]> = [
  ["Menús compuestos sin el editor: {count}.", "Menús compuestos sin el editor: {count}."],
  ["Codificación AVCHD con aceleración hardware: {}", "Codificación AVCHD con aceleración hardware: {encoder}"],
  ["«{}»: el vídeo se copia sin recodificar.", "«{title}»: el vídeo se copia sin recodificar."],
  ["«{}»: ya estaba codificado en un render anterior; se reutiliza.", "«{title}»: ya estaba codificado en un render anterior; se reutiliza."],
  ["Episodio AVCHD {}/{}: reutilizando caché de artefactos {}", "Episodio AVCHD {index}/{total}: reutilizando caché de artefactos {path}"],
  ["Codificando episodio AVCHD {}/{}: {} ({} kbps)", "Codificando episodio AVCHD {index}/{total}: {title} ({kbps} kbps)"],
  ["Reanudación: {reused_episodes} episodio(s) ya codificados en un render anterior se reutilizaron; solo se codificó lo que faltaba.", "Reanudación: {count} episodio(s) ya codificados en un render anterior se reutilizaron; solo se codificó lo que faltaba."],
  ["Multiplexando BDMV con el backend nativo de FFmpeg", "Multiplexando BDMV con el backend nativo de FFmpeg"],
  ["{} episodios con playlist propia: cada uno vuelve al menú al terminar", "{count} episodios con playlist propia: cada uno vuelve al menú al terminar"],
  ["First Play/Top Menu apuntan al objeto raíz {root_object} ({} menús)", "First Play/Top Menu apuntan al objeto raíz {object} ({count} menús)"],
  ["Intro: reutilizando caché {}", "Intro: reutilizando caché {path}"],
  ["Episodio {}: multiplexando subtítulo {} ({}, {} ms).", "Episodio {episode}: multiplexando subtítulo {index} ({language}, {delay} ms)."],
  ["Codificando {} episodios de a {} a la vez.", "Codificando {count} episodios de a {jobs} a la vez."],
  ["Codificando episodio {}/{}: {}", "Codificando episodio {index}/{total}: {title}"],
  ["Episodio {}/{}: reutilizando caché de artefactos {}", "Episodio {index}/{total}: reutilizando caché de artefactos {path}"],
  ["Usando composición del editor para el menú «{}».", "Usando composición del editor para el menú «{name}»."],
  ["Fondo del menú «{}» no encontrado; se usa color base.", "Fondo del menú «{name}» no encontrado; se usa color base."],
  ["Compilando menú {}/{}: {} (spumux)", "Compilando menú {index}/{total}: {name} (spumux)"],
  ["Menú {}/{}: reutilizando caché {}", "Menú {index}/{total}: reutilizando caché {path}"],
  ["Menú animado {}/{}: {} ({} botones de vídeo)", "Menú animado {index}/{total}: {name} ({count} botones de vídeo)"],
  ["Menú «{}»: falta labels-<id>.png; se renderiza estático.", "Menú «{name}»: falta labels-<id>.png; se renderiza estático."],
  ["Menú HDMV IG compilado: {} ({} bytes, {} PES, objeto {})", "Menú HDMV IG compilado: {name} ({bytes} bytes, {pes} PES, objeto {object})"],
  ["Menú HDMV IG «{}» adjuntado: playlist {} (clip {}, objeto {}, {} paquetes IG)", "Menú HDMV IG «{name}» adjuntado: playlist {playlist} (clip {clip}, objeto {object}, {packets} paquetes IG)"],
];

const LOG_PATTERNS = LOG_TEMPLATES.map(([template, key]) => {
  const source = template
    .split(/\{[^}]*\}/)
    .map((part) => part.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"))
    .join("(.+?)");
  const names = [...key.matchAll(/\{(\w+)\}/g)].map((match) => match[1]);
  return { pattern: new RegExp(`^${source}$`), key, names };
});

/** A line of the production log in the interface language. */
export function localizeLogLine(message: string): string {
  const text = message.trim();
  for (const { pattern, key, names } of LOG_PATTERNS) {
    const match = pattern.exec(text);
    if (!match) continue;
    const values: Record<string, string> = {};
    names.forEach((name, index) => (values[name] = match[index + 1] ?? ""));
    return t(key, values);
  }
  return i18n.has(text) ? t(text) : text;
}

/** What each structural check looks at (stable verifier IDs), for the result list. */
const CHECK_TITLES: Record<string, string> = {
  "UDF-VRS": "Secuencia de reconocimiento del volumen",
  "UDF-AVDP": "Puntero al descriptor del volumen",
  "UDF-PVD": "Descriptor principal del volumen",
  "UDF-PARTITION": "Partición del disco",
  "UDF-FSD": "Descriptor del conjunto de archivos",
  "UDF-ROOT": "Árbol de carpetas legible",
  "UDF-TAG266": "Entradas de archivo extendidas",
  "UDF-MIRROR": "Metadatos y su copia espejo",
  "UDF-EXTENT": "Tamaños de archivo dentro de la imagen",
  "UDF-DIGEST": "Huella SHA-256 de la imagen",
  "DVD-VOB": "Archivos de vídeo (VOB)",
  "DVD-IFO-BUP": "Archivos de control (IFO) y sus copias (BUP)",
  "DVD-VTS-IFO": "Conjunto de títulos",
  "DVD-PGC": "Programas y celdas de los títulos",
  "DVD-LAYER-BREAK": "Cambio de capa (DVD-9)",
  "DVD-REGION": "Regiones del disco",
  "DVD-AUDIO-STREAM": "Pista de audio en el vídeo",
  "DVD-SPU-SEQ": "Paquetes de botones y subtítulos",
  "DVD-SPU-PALETTE": "Colores de botones y subtítulos",
  "DVD-SPU-BUTTONS": "Botones del menú alcanzables",
  "BDMV-INDEX": "Índice del disco (index.bdmv)",
  "BDMV-MOBJ": "Programas de navegación (MovieObject)",
  "BDMV-MPLS": "Listas de reproducción",
  "BDMV-CLPI": "Información de los clips",
  "BDMV-M2TS": "Formato de los vídeos (M2TS)",
  "BDMV-PID": "Vídeo y audio declarados",
  "BDMV-AUDIO-STREAM": "Pista de audio",
  "BDMV-IG": "Menú interactivo",
  "BDMV-IG-BUTTONS": "Botones del menú",
  "BDMV-IG-NAV": "Navegación entre botones",
  "BDMV-MOTION-THUMB": "Menús con vídeo",
  "BDMV-PGS": "Subtítulos",
  "BDMV-PGS-DS": "Bloques de subtítulos",
  "BDMV-PGS-TS": "Tiempos de los subtítulos",
  "BDMV-MARKS": "Marcas de capítulo",
};

const CHECK_STATUS: Record<string, string> = {
  pass: "bien",
  warn: "aviso",
  fail: "error",
  skipped: "omitida",
};

/** "[ok] Readable folder tree" in the interface language; the detail stays as a tooltip. */
export function localizeCheck(check: { id: string; status: string; message: string }): string {
  const status = CHECK_STATUS[check.status] ? t(CHECK_STATUS[check.status]) : check.status;
  const title = CHECK_TITLES[check.id] ? t(CHECK_TITLES[check.id]) : check.id;
  return `[${status}] ${title}`;
}

/** Prefixes the render engine puts in front of its errors (RenderError). */
const RENDER_ERROR_PREFIXES: Array<[string, string]> = [
  ["error de E/S: ", "Error de lectura o escritura: {detail}"],
  ["no se pudo sondear los medios: ", "No se pudieron analizar los vídeos: {detail}"],
  ["el plan de bitrate no es viable: ", "El contenido no entra en el disco: {detail}"],
  ["error de codificación: ", "Error al codificar el vídeo: {detail}"],
  ["error de menú: ", "Error en los menús: {detail}"],
  ["error de authoring: ", "Error al crear la estructura del disco: {detail}"],
  ["error de ISO: ", "Error al crear la imagen ISO: {detail}"],
  ["error de imagen: ", "Error en una imagen: {detail}"],
];

/**
 * "el proyecto tiene errores: [E_X] error: …; [E_Y] error: …" from the planner,
 * as one sentence per problem in the interface language.
 */
export function localizeProjectErrors(message: string): string | null {
  const body = /^\s*el proyecto tiene errores:\s*([\s\S]*)$/.exec(message);
  if (!body) return null;
  const problems = [...body[1].matchAll(/\[([EW]_[A-Z0-9_]+)\]\s*(?:error|warning|aviso):\s*([^;]+)/g)].map(
    (match) => localizeDiagnostic({ code: match[1], message: match[2].trim() }),
  );
  const detail = problems.length > 0 ? problems.join(" ") : body[1];
  return t("El proyecto tiene un problema que impide generar el disco: {detail}", { detail });
}

/** A production error in the interface language (the engine writes Spanish). */
export function localizeRenderError(message: string): string {
  const text = message.trim();
  // A project error reached the planner: not a size problem.
  for (const [prefix] of RENDER_ERROR_PREFIXES) {
    if (text.startsWith(prefix)) {
      const project = localizeProjectErrors(text.slice(prefix.length));
      if (project) return project;
    }
  }
  const project = localizeProjectErrors(text);
  if (project) return project;
  for (const [prefix, key] of RENDER_ERROR_PREFIXES) {
    if (text.startsWith(prefix)) {
      const detail = text.slice(prefix.length);
      return t(key, { detail: i18n.has(detail) ? t(detail) : localizeError(detail) });
    }
  }
  if (text === "render cancelado") return t("Generación cancelada.");
  return i18n.has(text) ? t(text) : localizeError(text);
}

/**
 * Localizes the one-line result of a production job: the backend's
 * "ISO generado: …" summary, and the outcome kept in the job history.
 */
export function localizeJobSummary(summary: string): string {
  const dvd = /^ISO generado: (.+) \((\d+) bytes\)$/.exec(summary);
  if (dvd) return t("ISO generado: {path} ({size})", { path: dvd[1], size: formatBytes(Number(dvd[2])) });
  const hd = /^ISO AVCHD\/Blu-ray generado: (.+) \((\d+) bytes, UDF ([^,]+), (\d+) capítulos\)$/.exec(summary);
  if (hd) {
    return t("ISO AVCHD/Blu-ray generado: {path} ({size}, UDF {udf}, {chapters} capítulos)", {
      path: hd[1],
      size: formatBytes(Number(hd[2])),
      udf: hd[3],
      chapters: hd[4],
    });
  }
  if (i18n.has(summary)) return t(summary);
  return localizeRenderError(summary);
}
