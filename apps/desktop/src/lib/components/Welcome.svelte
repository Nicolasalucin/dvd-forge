<script lang="ts">
  import appLogo from "../../../src-tauri/icons/128x128@2x.png";
  import { onMount } from "svelte";
  import { commands, type ToolInstallOption } from "../api/bindings";
  import { messageOf, unwrap } from "../api/client";
  import * as actions from "../actions";
  import { t } from "../i18n/index.svelte";
  import { projectStore } from "../stores/project.svelte";
  import Icon from "../ui/Icon.svelte";
  import { formatBytes } from "../utils/format";
  import RecentProjects from "./RecentProjects.svelte";

  // First run: FFmpeg/DVDAuthor are downloaded and checked (SHA-256) by
  // Kinodisc itself; where there is no download, show the one command.
  let toInstall = $state<ToolInstallOption[]>([]);
  let manual = $state<ToolInstallOption[]>([]);
  let installing = $state<string | null>(null);
  let installError = $state<string | null>(null);
  let installed = $state(false);

  const totalBytes = $derived(toInstall.reduce((sum, option) => sum + (option.sizeBytes ?? 0), 0));

  onMount(() => {
    void checkTools();
  });

  async function checkTools() {
    try {
      const needed = await commands.toolchainNeedsInstall();
      toInstall = needed.length > 0 ? await commands.toolchainInstallOptions(needed) : [];
      const missing = (projectStore.toolsReport?.missingRequired ?? []).map((tool) => String(tool).toLowerCase());
      const missingManual = missing.filter((tool) => !needed.includes(tool));
      manual = missingManual.length > 0
        ? (await commands.toolchainInstallOptions(missingManual)).filter((option) => !option.automatic)
        : [];
    } catch {
      toInstall = [];
    }
  }

  async function installAll() {
    installError = null;
    try {
      for (const option of toInstall) {
        installing = option.tool;
        await unwrap(commands.toolchainInstall(option.tool));
      }
      installed = true;
      toInstall = [];
      await projectStore.refreshToolsDoctor();
    } catch (cause) {
      installError = messageOf(cause);
    } finally {
      installing = null;
    }
  }
</script>

<main class="flex min-h-0 flex-1 items-start justify-center overflow-y-auto p-8">
  <div class="w-full max-w-2xl">
    <div class="flex items-center gap-3">
      <img src={appLogo} alt="" width=48 height=48 class="shrink-0" draggable="false" />
      <div>
        <h1 class="text-[22px] font-semibold text-slate-100">{t("Bienvenido a Kinodisc")}</h1>
        <p class="text-[13px] text-slate-400">
          {t("Convierte tus vídeos en un DVD o Blu-ray con menú: importa, diseña el menú, pruébalo y grábalo.")}
        </p>
      </div>
    </div>

    {#if toInstall.length > 0}
      <section class="mt-6 rounded-lg border border-cyan-500/40 bg-cyan-500/5 p-4">
        <h2 class="text-[14px] font-semibold text-slate-100">{t("Falta preparar las herramientas de vídeo")}</h2>
        <p class="mt-1 text-[13px] leading-5 text-slate-300">
          {t("Kinodisc usa {tools} para convertir los vídeos. Se descargan una sola vez ({size}) y se comprueba que no hayan sido modificados.", {
            tools: toInstall.map((option) => (option.tool === "ffmpeg" ? "FFmpeg" : option.tool === "dvdauthor" ? "DVDAuthor" : option.tool)).join(", "),
            size: formatBytes(totalBytes),
          })}
        </p>
        <div class="mt-3 flex items-center gap-3">
          <button
            type="button"
            class="on-accent rounded-md bg-cyan-500 px-4 py-1.5 text-[13px] font-semibold hover:bg-cyan-400 disabled:opacity-60"
            onclick={installAll}
            disabled={installing !== null}
          >
            {installing ? t("Descargando {tool}…", { tool: installing }) : t("Descargar y preparar")}
          </button>
          {#if installError}<span class="text-[12px] text-rose-300" role="alert">{installError}</span>{/if}
        </div>
      </section>
    {:else if installed}
      <p class="mt-6 flex items-center gap-2 text-[13px] text-emerald-300" role="status">
        <Icon name="check" />{t("Herramientas listas. Ya puedes crear tu primer disco.")}
      </p>
    {/if}
    {#if manual.length > 0}
      <section class="mt-4 rounded-lg border border-amber-500/40 bg-amber-500/5 p-4">
        <h2 class="text-[14px] font-semibold text-slate-100">{t("Instala estas herramientas")}</h2>
        <ul class="mt-2 space-y-1 text-[13px] text-slate-300">
          {#each manual as option}
            <li><span class="font-medium">{option.tool}</span>{#if option.hint}: <code class="rounded bg-slate-800 px-1.5 py-0.5 text-[12px]">{option.hint}</code>{/if}</li>
          {/each}
        </ul>
      </section>
    {/if}

    <div class="mt-6 grid gap-3 sm:grid-cols-2">
      <button
        type="button"
        onclick={actions.newProject}
        class="flex items-start gap-3 rounded-lg border border-slate-700 bg-slate-900 p-4 text-left hover:border-cyan-500"
      >
        <Icon name="newFile" size={22} class="mt-0.5 text-cyan-400" />
        <span>
          <span class="block text-[14px] font-semibold text-slate-100">{t("Nuevo proyecto")}</span>
          <span class="mt-0.5 block text-[12px] text-slate-400">{t("Elige DVD o Blu-ray y añade tus vídeos")}</span>
        </span>
      </button>
      <button
        type="button"
        onclick={actions.openProject}
        class="flex items-start gap-3 rounded-lg border border-slate-700 bg-slate-900 p-4 text-left hover:border-cyan-500"
      >
        <Icon name="open" size={22} class="mt-0.5 text-cyan-400" />
        <span>
          <span class="block text-[14px] font-semibold text-slate-100">{t("Abrir proyecto…")}</span>
          <span class="mt-0.5 block text-[12px] text-slate-400">{t("Un proyecto o un paquete .kinodisc")}</span>
        </span>
      </button>
    </div>

    {#if projectStore.recentProjects.length > 0}
      <section class="mt-6">
        <h2 class="mb-2 text-[12px] font-semibold text-slate-400">{t("Proyectos recientes")}</h2>
        <RecentProjects compact />
      </section>
    {/if}

    <p class="mt-6 text-[12px] text-slate-500">
      {t("También puedes arrastrar un proyecto o tus vídeos a esta ventana.")}
      <button type="button" class="text-cyan-300 hover:underline" onclick={() => (projectStore.guideOpen = true)}>{t("Ver la guía paso a paso")}</button>
    </p>
  </div>
</main>
