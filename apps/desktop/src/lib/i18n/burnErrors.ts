/**
 * Why a burn or an erase failed, in plain words, plus what to do about it.
 *
 * Nothing here is sent anywhere: the explanation is shown in the dialog and
 * the technical details can be copied by the user (to ask for help, for
 * example). The engine writes Spanish; IMAPI errors keep their code
 * ("código 0xC0AA0202"), which is the most reliable thing to match.
 */
import { t } from "./index.svelte";

export interface BurnExplanation {
  /** Why it failed. */
  reason: string;
  /** What to try next. */
  advice: string;
  /** Erasing the disc would help (a used rewritable disc). */
  suggestErase: boolean;
}

type Entry = [reason: string, advice: string, suggestErase?: boolean];

const NO_DISC: Entry = ["No hay ningún disco en la grabadora.", "Pon un disco virgen o regrabable y vuelve a intentarlo."];
const WRONG_DISC: Entry = [
  "La grabadora no puede grabar este tipo de disco.",
  "Usa un DVD-R, DVD+R, DVD-RW o DVD+RW (o BD-R/BD-RE en una grabadora de Blu-ray).",
];
const NOT_EMPTY: Entry = [
  "El disco no está vacío o ya está cerrado.",
  "Usa un disco virgen. Si es regrabable (RW), bórralo primero.",
  true,
];
const STRANGE: Entry = [
  "La grabadora respondió de forma inesperada.",
  "Prueba con otro disco o con una velocidad menor.",
];

/** IMAPI2 codes (Windows) and what they mean. */
const IMAPI: Record<string, Entry> = {
  C0AA0002: ["Se canceló la operación.", "Puedes volver a empezar cuando quieras."],
  C0AA0007: ["El disco grabado no coincide con la imagen.", "Prueba con otro disco o con una velocidad menor."],
  C0AA0202: NO_DISC,
  C0AA0203: WRONG_DISC,
  C0AA0406: WRONG_DISC,
  C0AA0204: ["El disco está puesto al revés.", "Pon el disco con la cara brillante hacia abajo."],
  C0AA0205: ["La grabadora todavía está leyendo el disco.", "Espera unos segundos y vuelve a intentarlo."],
  C0AA0206: ["El disco se está formateando.", "Espera a que termine y vuelve a intentarlo."],
  C0AA0207: ["La grabadora está ocupada con otra tarea.", "Cierra otros programas que usen el disco y vuelve a intentarlo."],
  C0AA0209: NOT_EMPTY,
  C0AA0402: NOT_EMPTY,
  C0AA0405: NOT_EMPTY,
  C0AA0212: ["El disco regrabable no está formateado.", "Bórralo con «Borrar disco» y vuelve a intentarlo.", true],
  C0AA020D: [
    "La grabadora dejó de responder.",
    "Revisa el cable USB y su alimentación (en grabadoras externas, conecta los dos conectores o usa un hub con fuente).",
  ],
  C0AA020F: ["La velocidad elegida no sirve para este disco.", "Elige una velocidad menor."],
  C0AA0210: ["Otro programa tiene bloqueada la grabadora.", "Cierra otros programas de grabación y vuelve a intentarlo."],
  C0AA02FF: STRANGE,
  C0AA0301: STRANGE,
  C0AA0300: [
    "Los datos no llegaron a tiempo a la grabadora.",
    "Graba a una velocidad menor y evita tareas pesadas en la computadora mientras graba.",
  ],
  C0AA0404: ["La imagen no entra en este disco.", "Usa un disco de más capacidad o reparte el proyecto en varios discos."],
  C0AA0407: ["Esta grabadora no admite la operación.", "Prueba con otra grabadora."],
  C0AA090A: ["Esta grabadora no puede borrar discos.", "Prueba con otra grabadora."],
  C0AA0909: [
    "Este disco no se puede borrar.",
    "Solo se borran discos regrabables (DVD-RW, DVD+RW, BD-RE). Un DVD-R o DVD+R grabado no se puede volver a usar.",
  ],
};

/** Engine messages without a code (every system), matched by their start. */
const PHRASES: Array<[RegExp, Entry]> = [
  [/no hay medio insertado|no hay ningún disco/i, NO_DISC],
  [/no se detectaron unidades ópticas|no es un dispositivo óptico detectado/i, [
    "No se encontró la grabadora.",
    "Conéctala, espera unos segundos y pulsa «Actualizar».",
  ]],
  [/el dispositivo está ocupado|Ya hay una grabación en curso/i, [
    "La grabadora está ocupada.",
    "Espera a que termine lo que está haciendo y vuelve a intentarlo.",
  ]],
  [/verificación fallida/i, ["El disco grabado no coincide con la imagen.", "Prueba con otro disco o con una velocidad menor."]],
  [/Este disco no es regrabable|no puede borrar el disco/i, IMAPI.C0AA0909],
  [/permiso denegado/i, [
    "El sistema no dejó usar la grabadora.",
    "Cierra otros programas que la usen; en Linux, tu usuario necesita permiso sobre la unidad (grupo «cdrom» u «optical»).",
  ]],
  [/herramienta no encontrada/i, [
    "Falta un programa de grabación del sistema.",
    "En Linux instala «xorriso» o «growisofs»; en macOS se usa «drutil», que viene con el sistema.",
  ]],
  [/no permite borrar discos/i, ["Este sistema no permite borrar discos desde Kinodisc.", "Bórralo con la herramienta del sistema."]],
  [/medio no soportado/i, WRONG_DISC],
];

function explain(entry: Entry): BurnExplanation {
  return { reason: t(entry[0]), advice: t(entry[1]), suggestErase: entry[2] ?? false };
}

/** The explanation for a burn or erase error, or null when it is unknown. */
export function explainBurnError(message: string): BurnExplanation | null {
  const code = /0x(C0AA[0-9A-F]{4})/i.exec(message)?.[1]?.toUpperCase();
  if (code && IMAPI[code]) return explain(IMAPI[code]);
  for (const [pattern, entry] of PHRASES) {
    if (pattern.test(message)) return explain(entry);
  }
  return null;
}

/**
 * Technical details to copy when asking for help. Only what the user can
 * already see: no file paths beyond the image name, nothing sent.
 */
export function burnDiagnostics(details: {
  appVersion: string;
  action: string;
  device: string;
  image?: string | null;
  message: string;
}): string {
  const lines = [
    `Kinodisc ${details.appVersion}`,
    `${navigator.userAgent.match(/Windows NT [\d.]+|Mac OS X [\d_]+|Linux|FreeBSD/)?.[0] ?? "?"}`,
    `${details.action} · ${details.device || "?"}`,
  ];
  if (details.image) lines.push(details.image.replace(/^.*[\\/]/, ""));
  lines.push(details.message);
  return lines.join("\n");
}
