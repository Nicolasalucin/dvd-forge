<script lang="ts">
  import Icon from "../ui/Icon.svelte";
  import type { IconName } from "../ui/icons";
  import { t } from "../i18n/index.svelte";
  import { projectStore, type StudioMode } from "../stores/project.svelte";

  interface Props {
    onClose: () => void;
    onNewProject: () => void;
  }

  let { onClose, onNewProject }: Props = $props();

  interface Step {
    icon: IconName;
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
      icon: "info",
      title: "Bienvenido a Kinodisc",
      body: "Con Kinodisc conviertes tus vídeos en un disco con menú, como los de las películas. Son 6 pasos y esta guía te acompaña en cada uno.",
      tips: [
        "Puedes abrir esta guía cuando quieras con F1 o con el botón de ayuda de la barra de herramientas.",
        "Todo se guarda en un archivo de proyecto; tus vídeos originales nunca se modifican.",
      ],
    },
    {
      icon: "disc",
      title: "1 · Elige el tipo de disco",
      body: "DVD funciona en casi cualquier reproductor y consola. Blu-ray/AVCHD tiene imagen en alta definición (HD) y necesita un reproductor Blu-ray, PS3/PS4/PS5 o similar.",
      tips: [
        "¿Dudas? Elige DVD-Video: es el más compatible.",
        "El botón del disco, a la derecha de las pestañas, cambia el formato, la capacidad y el sistema de TV cuando quieras.",
      ],
      action: { label: "Crear un proyecto nuevo", run: () => onNewProject() },
    },
    {
      icon: "film",
      title: "2 · Añade tus vídeos",
      body: "Pulsa «Importar» o arrastra los vídeos a la ventana: cada uno es un episodio en la tira de abajo, que se ordena arrastrando.",
      tips: [
        "En Vídeos recortas el principio y el final y pones capítulos: cada 5 o 10 minutos, en 4 partes o «Por escenas», automáticos.",
        "«Disco › Unir vídeos…» junta varios clips (las partes de una filmación) en un episodio; «Fotos» crea una presentación con música.",
      ],
      action: { label: "Abrir Vídeos", run: () => goTo("trim", { mediaTab: "media" }) },
    },
    {
      icon: "photo",
      title: "3 · Arregla la imagen",
      body: "En Vídeos, la tarjeta «Imagen» corrige cada vídeo antes de grabarlo: girar, recortar bordes negros, brillo y color, ruido y desentrelazado.",
      tips: [
        "Los vídeos verticales del celular pueden llevar un fondo desenfocado en vez de barras negras.",
        "«Probar calidad» codifica 10 segundos igual que en el disco y los compara con el original.",
      ],
      action: { label: "Abrir Vídeos", run: () => goTo("trim", { mediaTab: "media" }) },
    },
    {
      icon: "menu",
      title: "4 · Diseña el menú",
      body: "En Menú ves la pantalla que aparecerá en la tele. Lo más fácil: pulsa «Plantillas» y elige una de las 15; Kinodisc crea un botón por episodio.",
      tips: [
        "«Texto» añade títulos; el selector de fuentes muestra cada letra antes de elegirla.",
        "Con clic derecho sobre un botón: duplicar, centrar, alinear o distribuir. «Fondo» y «Música» cambian el fondo y el sonido.",
      ],
      action: { label: "Ir al editor de menú", run: () => goTo("menu", { mediaTab: "layouts" }) },
    },
    {
      icon: "remote",
      title: "5 · Pruébalo con el mando",
      body: "Vista previa muestra el disco tal como funcionará en el reproductor. Usa las flechas del teclado y Enter, o el mando de la pantalla.",
      tips: [
        "Si un botón no se puede alcanzar con las flechas, la vista previa te lo avisa.",
        "En «Audio y subtítulos» eliges idiomas de audio y añades subtítulos (.srt, .ass, .vtt).",
      ],
      action: { label: "Abrir la vista previa", run: () => goTo("simulator") },
    },
    {
      icon: "burn",
      title: "6 · Crea el disco",
      body: "Pulsa «Producir disco» (arriba a la derecha): ISO, carpeta VIDEO_TS/BDMV, varios discos o grabarlo directamente.",
      tips: [
        "La barra de abajo indica cuánto ocupa el disco. Si se pasa, prueba un disco de doble capa o reparte en varios discos.",
        "Marca «Apagar la computadora al terminar» para trabajos largos; Kinodisc avisa con un sonido al acabar.",
        "«Disco › Carátula y etiqueta…» prepara la tapa de la caja y la etiqueta para imprimir.",
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
  class="fixed bottom-4 right-4 z-50 w-[min(420px,calc(100vw-2rem))] overflow-hidden rounded-lg border border-slate-700 bg-slate-900 shadow-xl"
  role="dialog"
  aria-modal="false"
  aria-labelledby="guide-title"
>
  <div class="flex items-start gap-3 p-4">
    <div class="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-cyan-500/15 text-cyan-400" aria-hidden="true">
      <Icon name={step.icon} size={20} />
    </div>
    <div class="min-w-0 flex-1">
      <p class="ui-section-title">
        {t("Guía · paso {current} de {total}", { current: index + 1, total: steps.length })}
      </p>
      <h2 id="guide-title" class="mt-0.5 text-[15px] font-semibold text-slate-50">{t(step.title)}</h2>
      <p class="mt-1.5 text-[12px] leading-5 text-slate-300">{t(step.body)}</p>
      <ul class="mt-2 space-y-1">
        {#each step.tips as tip}
          <li class="flex gap-1.5 text-[12px] leading-5 text-slate-400">
            <span class="text-cyan-400" aria-hidden="true">•</span><span>{t(tip)}</span>
          </li>
        {/each}
      </ul>
      {#if step.action}
        <button
          type="button"
          onclick={step.action.run}
          class="ui-btn mt-3"
        >
          {t(step.action.label)} →
        </button>
      {/if}
    </div>
    <button
      type="button"
      onclick={finish}
      class="ui-btn ui-btn-ghost"
      title={t("Cerrar la guía")}
      aria-label={t("Cerrar la guía")}
    >
      ✕
    </button>
  </div>
  <div class="flex items-center justify-between gap-2 border-t border-slate-800 px-4 py-2.5">
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
          class="rounded-lg px-2.5 py-1 text-[12px] text-slate-400 transition hover:bg-slate-800 hover:text-slate-200"
        >
          {t("Atrás")}
        </button>
      {/if}
      {#if last}
        <button
          type="button"
          onclick={finish}
          class="ui-btn ui-btn-sm ui-btn-primary"
        >
          {t("¡Entendido!")}
        </button>
      {:else}
        <button
          type="button"
          onclick={() => (index += 1)}
          class="rounded-lg bg-cyan-500 px-3 py-1 text-[12px] font-bold text-slate-950 transition hover:bg-cyan-400"
        >
          {t("Siguiente")}
        </button>
      {/if}
    </div>
  </div>
</div>
