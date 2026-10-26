import { t } from "./index.svelte";

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
    values: (match) => ({ percent: match[1] }),
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
    values: (match) => ({ percent: match[1], capacity: match[2] }),
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
  const rule = PATTERNS.find((candidate) =>
    candidate.pattern.test(diagnostic.message),
  );
  if (rule) {
    const match = diagnostic.message.match(rule.pattern);
    if (match) return t(rule.key, rule.values(match));
  }
  const fixed = FIXED[diagnostic.code];
  if (fixed) return t(fixed);
  return t(diagnostic.message);
}

const JOB_STAGES = new Set([
  "Sondeando medios",
  "Calculando bitrate",
  "Codificando intro",
  "Codificando episodios",
  "Compilando menús con spumux",
  "Compilando DVD con dvdauthor",
  "Construyendo ISO",
  "Calculando bitrate AVCHD",
  "Codificando episodios AVCHD",
  "Multiplexando AVCHD con tsMuxeR",
  "Multiplexando BDMV con menús HDMV IG",
  "Validando ISO AVCHD",
]);

export function localizeJobStage(label: string): string {
  return JOB_STAGES.has(label) ? t(label) : label;
}
