#!/usr/bin/env node
// Reports UI strings that are missing from the locale catalogs.
//
//   node scripts/i18n-check.mjs            # summary + missing English keys
//   node scripts/i18n-check.mjs --json     # machine-readable report
//   node scripts/i18n-check.mjs --strict   # exit 1 when English is incomplete
//
// Keys are the Spanish source strings passed to t("…") plus the `label`,
// `title` and `hint` literals of option tables that are later rendered
// through t(option.label). Spanish is the source language; every other
// locale falls back to English, so English must always be complete.
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const src = join(root, "src");

function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) walk(path, out);
    else if (/\.(svelte|ts)$/.test(name) && !name.endsWith("bindings.ts") && !path.includes(`${join("lib", "i18n")}`)) {
      out.push(path);
    }
  }
  return out;
}

function unescape(literal) {
  return literal.replace(/\\(["'`\\])/g, "$1").replace(/\\n/g, "\n");
}

const keys = new Map();
const add = (key, file) => {
  if (!key || !/[A-Za-zÁÉÍÓÚÑáéíóúñ]/.test(key)) return;
  if (!keys.has(key)) keys.set(key, new Set());
  keys.get(key).add(relative(root, file));
};

const tCall = /\bt\(\s*(["'])((?:\\.|(?!\1).)*)\1/g;
const tTernary = /\bt\(\s*[^()"'`]*?\?\s*(["'])((?:\\.|(?!\1).)*)\1\s*:\s*(["'])((?:\\.|(?!\3).)*)\3/g;
const tableLiteral = /\b(label|title|hint|description|name)\s*:\s*(["'])((?:\\.|(?!\2).)*)\2/g;

for (const file of walk(src)) {
  const text = readFileSync(file, "utf8");
  for (const match of text.matchAll(tCall)) add(unescape(match[2]), file);
  for (const match of text.matchAll(tTernary)) {
    add(unescape(match[2]), file);
    add(unescape(match[4]), file);
  }
  // Option tables rendered via t(x.label) / t(x.title) / t(x.hint).
  if (/\bt\(\s*[a-zA-Z_]+\.(label|title|hint|description)\s*\)/.test(text)) {
    for (const match of text.matchAll(tableLiteral)) add(unescape(match[3]), file);
  }
}

// Load catalogs without a TS toolchain: the file is a set of object literals.
const catalogSource = readFileSync(join(src, "lib", "i18n", "messages.ts"), "utf8");
const catalogs = {};
const block = /export const (\w+): Dictionary = \{([\s\S]*?)\n\};/g;
const entry = /^\s*("(?:\\.|[^"\\])*")\s*:\s*("(?:\\.|[^"\\])*"),?\s*$/gm;
for (const match of catalogSource.matchAll(block)) {
  const dictionary = {};
  for (const item of match[2].matchAll(entry)) {
    dictionary[JSON.parse(item[1])] = JSON.parse(item[2]);
  }
  catalogs[match[1]] = dictionary;
}

const report = {};
for (const [locale, dictionary] of Object.entries(catalogs)) {
  if (locale === "es") continue;
  report[locale] = [...keys.keys()].filter((key) => !(key in dictionary)).sort();
}

// Every diagnostic code emitted by Rust needs an English meaning, otherwise a
// non-Spanish UI would show the raw Spanish message.
const codeTable = readFileSync(join(src, "lib", "i18n", "diagnosticCodes.ts"), "utf8");
const knownCodes = new Set([...codeTable.matchAll(/^\s*([EW]_[A-Z0-9_]+):/gm)].map((m) => m[1]));
const rustCodes = new Set();
function walkRust(dir) {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) {
      if (name !== "target" && name !== "node_modules") walkRust(path);
    } else if (name.endsWith(".rs")) {
      for (const m of readFileSync(path, "utf8").matchAll(/"([EW]_[A-Z0-9_]+)"/g)) rustCodes.add(m[1]);
    }
  }
}
walkRust(join(root, "..", "..", "crates"));
walkRust(join(root, "src-tauri", "src"));
const missingCodes = [...rustCodes].filter((code) => !knownCodes.has(code)).sort();

// Per-locale diagnostic tables (pt/fr/de/it/ja) must cover every code too.
const localeTables = readFileSync(join(src, "lib", "i18n", "diagnosticCodesI18n.ts"), "utf8");
const missingLocaleCodes = {};
for (const match of localeTables.matchAll(/export const DIAGNOSTIC_(\w+): Record<string, string> = \{([\s\S]*?)\n\};/g)) {
  const present = new Set([...match[2].matchAll(/^\s*([EW]_[A-Z0-9_]+):/gm)].map((m) => m[1]));
  missingLocaleCodes[match[1].toLowerCase()] = [...rustCodes].filter((code) => !present.has(code)).sort();
}
const localeCodeGaps = Object.values(missingLocaleCodes).reduce((sum, list) => sum + list.length, 0);

const args = new Set(process.argv.slice(2));
if (args.has("--json")) {
  console.log(JSON.stringify({ keys: keys.size, missing: report, missingCodes, missingLocaleCodes }, null, 2));
} else {
  console.log(`UI keys: ${keys.size}`);
  for (const [locale, missing] of Object.entries(report)) {
    console.log(`  ${locale}: ${missing.length} missing${locale === "en" ? "" : " (falls back to English)"}`);
  }
  if (report.en?.length) {
    console.log("\nMissing English keys:");
    for (const key of report.en) console.log(`  ${JSON.stringify(key)}  ← ${[...keys.get(key)].join(", ")}`);
  }
}
if (!args.has("--json")) {
  console.log(`Diagnostic codes: ${rustCodes.size} in Rust, ${missingCodes.length} without English text`);
  for (const code of missingCodes) console.log(`  ${code}`);
  for (const [locale, list] of Object.entries(missingLocaleCodes)) {
    console.log(`  ${locale}: ${list.length} codes without a ${locale} text${list.length ? ": " + list.join(", ") : ""}`);
  }
}
const uiGaps = Object.values(report).reduce((sum, list) => sum + list.length, 0);
if (args.has("--strict") && (uiGaps || missingCodes.length || localeCodeGaps)) process.exit(1);
