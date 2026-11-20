<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { videoDir } from "@tauri-apps/api/path";
  import { onMount } from "svelte";
  import { commands } from "../api/bindings";
  import { messageOf, unwrap } from "../api/client";
  import { t } from "../i18n/index.svelte";
  import { volumeLabelFor } from "../stores/project.svelte";
  import Icon from "../ui/Icon.svelte";
  import { basename, formatBytes, nativePath } from "../utils/format";

  let { onclose, onburn }: { onclose: () => void; onburn: (isoPath: string) => void } = $props();

  // Nominal capacities, smallest first: the dialog names the first that fits.
  const DISCS: Array<[string, number]> = [
    ["CD", 700_000_000],
    ["DVD-5", 4_700_000_000],
    ["DVD-9", 8_540_000_000],
    ["BD-25", 25_000_000_000],
    ["BD-50", 50_000_000_000],
  ];
  const OUTPUT_KEY = "kinodisc_output_dir";

  let folder = $state("");
  let label = $state("");
  let outputDir = $state("");
  let size = $state<{ files: number; bytes: number } | null>(null);
  let working = $state(false);
  let error = $state<string | null>(null);
  let created = $state<{ isoPath: string; bytes: number } | null>(null);

  // Room for the file system itself: about 1 % plus 2 MB.
  const needed = $derived(size ? size.bytes * 1.01 + 2_000_000 : 0);
  const fits = $derived(size ? (DISCS.find(([, capacity]) => needed <= capacity) ?? null) : null);
  const canCreate = $derived(!working && folder !== "" && outputDir.trim() !== "" && size !== null && size.files > 0);

  onMount(async () => {
    try {
      outputDir = localStorage.getItem(OUTPUT_KEY) ?? "";
    } catch {
      outputDir = "";
    }
    if (!outputDir) {
      try {
        outputDir = await videoDir();
      } catch {
        outputDir = "";
      }
    }
  });

  async function pickFolder() {
    const picked = await open({ directory: true, multiple: false });
    if (typeof picked !== "string") return;
    folder = picked;
    created = null;
    error = null;
    size = null;
    if (!label.trim()) label = volumeLabelFor(basename(picked));
    try {
      const measured = await unwrap(commands.dataFolderSize(picked));
      size = { files: measured.files, bytes: measured.bytes ?? 0 };
    } catch (cause) {
      error = messageOf(cause);
    }
  }

  async function pickOutput() {
    const picked = await open({ directory: true, multiple: false });
    if (typeof picked === "string") outputDir = picked;
  }

  async function create() {
    if (!canCreate) return;
    working = true;
    error = null;
    created = null;
    try {
      const target = nativePath(outputDir.trim());
      const outcome = await unwrap(commands.dataIsoCreate(folder, target, label));
      created = { isoPath: outcome.isoPath, bytes: outcome.bytes ?? 0 };
      try {
        localStorage.setItem(OUTPUT_KEY, target);
      } catch {
        // Not remembered this time.
      }
    } catch (cause) {
      error = messageOf(cause);
    } finally {
      working = false;
    }
  }

  async function cancel() {
    if (working) {
      await commands.dataIsoCancel();
      return;
    }
    onclose();
  }
</script>

<div class="fixed inset-0 z-50 flex items-center justify-center p-4">
  <button type="button" class="ui-backdrop" onclick={cancel} aria-label={t("Cerrar")}></button>
  <div class="ui-dialog max-w-xl" role="dialog" aria-modal="true" aria-labelledby="data-disc-title">
    <header class="flex items-start gap-3 border-b border-slate-800 px-5 py-4">
      <Icon name="folder" size={20} class="mt-0.5 text-cyan-400" />
      <div>
        <h2 id="data-disc-title" class="text-[15px] font-semibold text-slate-100">{t("Disco de datos")}</h2>
        <p class="ui-hint mt-0.5">
          {t("Graba cualquier carpeta (fotos, documentos, originales) en un CD, DVD o Blu-ray que se abre en la computadora. No es un disco de vídeo para la tele.")}
        </p>
      </div>
    </header>

    <div class="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-5 py-4">
      <div>
        <span class="ui-label">{t("Carpeta que quieres grabar")}</span>
        <div class="flex items-center gap-2">
          <input class="ui-input w-full font-mono" value={folder} readonly placeholder={t("Ninguna carpeta elegida")} />
          <button type="button" class="ui-btn shrink-0" onclick={pickFolder} disabled={working}>{t("Elegir…")}</button>
        </div>
        {#if size}
          <p class="mt-1 text-[12px] {fits ? 'text-slate-400' : 'text-rose-300'}">
            {t("{count} archivos · {size}", { count: size.files, size: formatBytes(size.bytes) })}
            ·
            {fits ? t("Entra en un {disc}.", { disc: `${fits[0]} (${formatBytes(fits[1])})` }) : t("No entra en un solo disco; reparte la carpeta en varias.")}
          </p>
        {/if}
      </div>

      <label class="block">
        <span class="ui-label">{t("Etiqueta del disco")}</span>
        <input
          class="ui-input w-full font-mono uppercase"
          maxlength="30"
          bind:value={label}
          onchange={() => (label = volumeLabelFor(label || basename(folder)))}
          disabled={working}
        />
      </label>

      <div>
        <span class="ui-label">{t("Carpeta de salida")}</span>
        <div class="flex items-center gap-2">
          <input class="ui-input w-full font-mono" bind:value={outputDir} disabled={working} />
          <button type="button" class="ui-btn shrink-0" onclick={pickOutput} disabled={working}>{t("Elegir…")}</button>
        </div>
        <p class="ui-hint mt-1">{t("Aquí se guarda la imagen .iso del disco; después puedes grabarla.")}</p>
      </div>

      {#if working}
        <p class="rounded-md bg-cyan-500/10 px-3 py-2 text-[12px] text-cyan-200" role="status">
          {t("Creando la imagen del disco… Con carpetas grandes puede tardar varios minutos.")}
        </p>
      {/if}
      {#if created}
        <div class="rounded-md border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-[12px] text-emerald-200" role="status">
          <p>{t("Imagen creada: {name} ({size})", { name: basename(created.isoPath), size: formatBytes(created.bytes) })}</p>
          <div class="mt-2 flex flex-wrap gap-2">
            <button type="button" class="ui-btn ui-btn-sm ui-btn-primary" onclick={() => created && onburn(created.isoPath)}>
              {t("Grabar en un disco…")}
            </button>
            <button type="button" class="ui-btn ui-btn-sm" onclick={() => created && void commands.openFolder(created.isoPath)}>
              {t("Abrir carpeta")}
            </button>
          </div>
        </div>
      {/if}
      {#if error}
        <p class="rounded-md bg-rose-500/10 px-3 py-2 text-[12px] text-rose-200" role="alert">{error}</p>
      {/if}
    </div>

    <footer class="ui-dialog-footer">
      <button type="button" class="ui-btn ui-btn-ghost" onclick={cancel}>{working ? t("Cancelar") : t("Cerrar")}</button>
      <button type="button" class="ui-btn ui-btn-primary" onclick={create} disabled={!canCreate}>
        {working ? t("Creando…") : t("Crear imagen del disco")}
      </button>
    </footer>
  </div>
</div>
