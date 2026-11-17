<script lang="ts">
  import { t } from "../i18n/index.svelte";
  import { projectStore, type StudioMode } from "../stores/project.svelte";

  interface Props {
    onClose: () => void;
    onNewProject: () => void;
  }

  let { onClose, onNewProject }: Props = $props();

  interface Step {
    icon: string;
    title: string;
    body: string;
    tips: string[];
    action?: { label: string; run: () => void };
  }

  function goTo(mode: StudioMode, options: { mediaTab?: typeof projectStore.mediaTab } = {}) {
    if (!projectStore.project) {
      onNewProject();
      return;
    }
    projectStore.studioMode = mode;
    if (options.mediaTab) {
      projectStore.mediaTab = options.mediaTab;
      projectStore.leftCollapsed = false;
    }
  }

  const steps: Step[] = [
    {
      icon: "👋",
      title: "Bienvenido a Kinodisc",
      body: "Con Kinodisc conviertes tus vídeos en un disco con menú, como los de las películas. Son 5 pasos y esta guía te acompaña en cada uno.",
      tips: [
        "Puedes abrir esta guía cuando quieras con el botón ❓ Guía de arriba.",
        "Todo se guarda en un archivo de proyecto; tus vídeos originales nunca se modifican.",
      ],
    },
    {
      icon: "💿",
      title: "1 · Elige el tipo de disco",
      body: "DVD funciona en casi cualquier reproductor y consola. Blu-ray/AVCHD tiene imagen en alta definición (HD) y necesita un reproductor Blu-ray, PS3/PS4/PS5 o similar.",
      tips: [
        "¿Dudas? Elige DVD-Video: es el más compatible.",
        "Para Blu-ray, usa 1080p. Si tus vídeos son de TV o de un móvil europeo elige 25 fps; si son de EE. UU./América, 29,97 fps.",
      ],
      action: { label: "Crear un proyecto nuevo", run: () => onNewProject() },
    },
    {
      icon: "🎞",
      title: "2 · Añade tus vídeos",
      body: "En el panel izquierdo (pestaña «Medios») pulsa «+ Importar» y elige tus vídeos. Después pulsa «+ Episodio» en cada uno para ponerlo en el disco.",
      tips: [
        "Los episodios aparecen abajo, en la barra de Secuencia, en el orden en que se reproducirán. Usa ‹ › para reordenarlos.",
        "En Edición puedes recortar el principio y el final de cada vídeo y añadir capítulos.",
      ],
      action: { label: "Abrir Medios", run: () => goTo("trim", { mediaTab: "media" }) },
    },
    {
      icon: "🎨",
      title: "3 · Diseña el menú",
      body: "En la pestaña Menú ves la pantalla que aparecerá en la tele. Lo más fácil: abre la pestaña «Plantillas» del panel izquierdo (o la galería debajo del menú) y elige una; Kinodisc crea un botón por episodio.",
      tips: [
        "Para cambiar el fondo, elige una imagen en «Medios» y pulsa «Fondo».",
        "Para la miniatura de un botón: selecciónalo y pulsa «Usar imagen de…» en el Inspector (derecha).",
        "¿Quieres otra pantalla de menú? Pulsa «+ Pantalla de menú» en la barra de abajo.",
      ],
      action: { label: "Ir al editor de menú", run: () => goTo("menu", { mediaTab: "layouts" }) },
    },
    {
      icon: "🎮",
      title: "4 · Pruébalo con el mando",
      body: "El Simulador muestra el disco tal como funcionará en el reproductor. Usa las flechas del teclado y Enter, o el mando de la pantalla.",
      tips: [
        "Si un botón no se puede alcanzar con las flechas, el simulador te lo avisa.",
        "En Pistas puedes elegir idiomas de audio y añadir subtítulos (.srt, .ass, .vtt).",
      ],
      action: { label: "Abrir el simulador", run: () => goTo("simulator") },
    },
    {
      icon: "🔥",
      title: "5 · Crea el disco",
      body: "Pulsa «Producir disco» (arriba a la derecha). Kinodisc prepara un archivo ISO y, si tienes grabadora, puede grabarlo directamente.",
      tips: [
        "La barra de capacidad indica cuánto ocupa el disco. Si se pasa, prueba un disco de doble capa (DVD-9 o BD-50).",
        "Antes de grabar, el disco se comprueba automáticamente para evitar discos defectuosos.",
        "Si algo falla, el botón ⚕ Salud te dice qué herramienta falta (por ejemplo, FFmpeg).",
      ],
      action: {
        label: "Abrir «Producir disco»",
        run: () => {
          if (!projectStore.project) onNewProject();
          else projectStore.productionOpen = true;
        },
      },
    },
  ];

  let index = $state(0);
  const step = $derived(steps[index]);
  const last = $derived(index === steps.length - 1);

  function finish() {
    try {
      window.localStorage.setItem("kinodisc_guide_done", "1");
    } catch {
      // Best effort.
    }
    onClose();
  }

  function onKeydown(event: KeyboardEvent) {
    if (event.key === "Escape") finish();
    else if (event.key === "ArrowRight" && !last) index += 1;
    else if (event.key === "ArrowLeft" && index > 0) index -= 1;
  }
</script>

<svelte:window onkeydown={onKeydown} />

<div
  class="fixed bottom-4 right-4 z-50 w-[min(420px,calc(100vw-2rem))] overflow-hidden rounded-2xl border border-cyan-500/30 bg-slate-950/95 shadow-2xl shadow-cyan-950/40 backdrop-blur"
  role="dialog"
  aria-modal="false"
  aria-labelledby="guide-title"
>
  <div class="flex items-start gap-3 p-4">
    <div class="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl bg-cyan-500/15 text-2xl" aria-hidden="true">
      {step.icon}
    </div>
    <div class="min-w-0 flex-1">
      <p class="text-[10px] font-semibold uppercase tracking-[0.18em] text-cyan-300/80">
        {t("Guía · paso {current} de {total}", { current: index + 1, total: steps.length })}
      </p>
      <h2 id="guide-title" class="mt-0.5 text-base font-semibold text-white">{t(step.title)}</h2>
      <p class="mt-1.5 text-[12px] leading-5 text-slate-300">{t(step.body)}</p>
      <ul class="mt-2 space-y-1">
        {#each step.tips as tip}
          <li class="flex gap-1.5 text-[11px] leading-5 text-slate-400">
            <span class="text-cyan-400" aria-hidden="true">•</span><span>{t(tip)}</span>
          </li>
        {/each}
      </ul>
      {#if step.action}
        <button
          type="button"
          onclick={step.action.run}
          class="mt-3 rounded-lg border border-cyan-500/50 bg-cyan-500/10 px-3 py-1.5 text-[11px] font-semibold text-cyan-200 transition hover:bg-cyan-500/20"
        >
          {t(step.action.label)} →
        </button>
      {/if}
    </div>
    <button
      type="button"
      onclick={finish}
      class="rounded-md px-1.5 text-sm text-slate-500 transition hover:bg-slate-800 hover:text-slate-200"
      title={t("Cerrar la guía")}
      aria-label={t("Cerrar la guía")}
    >
      ✕
    </button>
  </div>
  <div class="flex items-center justify-between gap-2 border-t border-slate-800 bg-slate-900/60 px-4 py-2.5">
    <div class="flex gap-1" aria-hidden="true">
      {#each steps as _, dot}
        <button
          type="button"
          tabindex="-1"
          aria-label={t("Ir al paso {number}", { number: dot + 1 })}
          onclick={() => (index = dot)}
          class={`h-1.5 rounded-full transition-all ${dot === index ? "w-5 bg-cyan-400" : "w-1.5 bg-slate-700 hover:bg-slate-500"}`}
        ></button>
      {/each}
    </div>
    <div class="flex items-center gap-1.5">
      {#if index > 0}
        <button
          type="button"
          onclick={() => (index -= 1)}
          class="rounded-lg px-2.5 py-1 text-[11px] text-slate-400 transition hover:bg-slate-800 hover:text-slate-200"
        >
          {t("Atrás")}
        </button>
      {/if}
      {#if last}
        <button
          type="button"
          onclick={finish}
          class="rounded-lg bg-cyan-500 px-3 py-1 text-[11px] font-bold text-slate-950 transition hover:bg-cyan-400"
        >
          {t("¡Entendido!")}
        </button>
      {:else}
        <button
          type="button"
          onclick={() => (index += 1)}
          class="rounded-lg bg-cyan-500 px-3 py-1 text-[11px] font-bold text-slate-950 transition hover:bg-cyan-400"
        >
          {t("Siguiente")}
        </button>
      {/if}
    </div>
  </div>
</div>
