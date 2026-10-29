<script lang="ts">
  import { t } from "../i18n/index.svelte";
  import type { Menu_Serialize } from "../api/bindings";
  import { projectStore } from "../stores/project.svelte";
  import { formatDuration } from "../utils/format";

  type NodeKind = "firstPlay" | "vmgm" | "vtsm" | "episode" | "continuous";

  interface MapNode {
    id: string;
    kind: NodeKind;
    title: string;
    subtitle: string;
    x: number;
    y: number;
    width: number;
    menuId?: string;
    episodeId?: string;
  }

  interface MapEdge {
    id: string;
    from: string;
    to: string;
    label: string;
    dashed: boolean;
  }

  const project = $derived(projectStore.project);
  const isAvchd = $derived(project?.disc.mode === "avchd");
  const isContinuousAvchd = $derived(isAvchd && (project?.menus.length ?? 0) === 0);
  const rootMenu = $derived(project?.menus.find((menu) => menu.id === project?.rootMenu) ?? null);
  let selectedId = $state<string | null>(null);

  function roleLabel(menu: Menu_Serialize): string {
    const role =
      menu.role === "chapterSelect"
        ? t("Capítulos")
        : menu.role === "episodeSelect"
          ? t("Episodios")
          : menu.role === "submenu"
            ? t("Submenú")
            : t("Raíz");
    return menu.placement.kind === "vtsm"
      ? `VTSM ${menu.placement.titleset} · ${role}`
      : `VMGM · ${role}`;
  }

  function actionLabel(kind: string): string {
    switch (kind) {
      case "playEpisode":
        return t("reproducir");
      case "playAll":
        return t("reproducir todo");
      case "gotoMenu":
        return t("ir a menú");
      case "jumpTitleChapter":
        return t("capítulo");
      case "playFirstPlay":
        return t("intro");
      case "setAudioTrack":
        return t("pista de audio");
      case "resume":
        return t("reanudar");
      default:
        return t("sin acción");
    }
  }

  function flowColumns(hasFirstPlay: boolean): {
    vmgm: number;
    vtsm: number;
    episode: number;
    width: number;
  } {
    const vmgm = hasFirstPlay ? 170 : 30;
    const vtsm = vmgm + 170;
    const episode = vtsm + 185;
    return { vmgm, vtsm, episode, width: episode + 177 };
  }

  function buildNodes(): MapNode[] {
    const current = project;
    if (!current) return [];
    const nodes: MapNode[] = [];
    if (isContinuousAvchd) {
      nodes.push({
        id: "continuous",
        kind: "continuous",
        title: t("REPRODUCCIÓN CONTINUA"),
        subtitle: `${current.episodes.length} ${t(current.episodes.length === 1 ? "título" : "títulos")} · ${t("sin menús")}`,
        x: 16,
        y: 32,
        width: 180,
      });
      current.episodes.forEach((episode, index) => {
        nodes.push({
          id: `ep-${episode.id}`,
          kind: "episode",
          title: `${t("Título")} ${index + 1} · ${episode.title}`,
          subtitle: `${formatDuration(projectStore.episodeEffectiveDuration(episode))} · ${episode.chapters.length} ${t("cap.")}`,
          x: 260,
          y: 32 + index * 74,
          width: 230,
          episodeId: episode.id,
        });
      });
      return nodes;
    }

    const vmgm = current.menus.filter((menu) => menu.placement.kind === "vmgm");
    const vtsm = current.menus.filter((menu) => menu.placement.kind === "vtsm");
    const columns = flowColumns(Boolean(current.firstPlay));
    if (current.firstPlay) {
      nodes.push({
        id: "firstPlay",
        kind: "firstPlay",
        title: t("INICIO"),
        subtitle: projectStore.assetFor(current.firstPlay.asset)?.displayName ?? t("Intro"),
        x: 10,
        y: 32,
        width: 120,
      });
    }
    vmgm.forEach((menu, index) => {
      nodes.push({
        id: `menu-${menu.id}`,
        kind: "vmgm",
        title: menu.name,
        subtitle: roleLabel(menu),
        x: columns.vmgm,
        y: 32 + index * 74,
        width: 130,
        menuId: menu.id,
      });
    });
    vtsm.forEach((menu, index) => {
      nodes.push({
        id: `menu-${menu.id}`,
        kind: "vtsm",
        title: menu.name,
        subtitle: roleLabel(menu),
        x: columns.vtsm,
        y: 32 + index * 74,
        width: 145,
        menuId: menu.id,
      });
    });
    current.episodes.forEach((episode, index) => {
      nodes.push({
        id: `ep-${episode.id}`,
        kind: "episode",
        title: `${t("Título")} ${index + 1} · ${episode.number.toString().padStart(2, "0")}`,
        subtitle: `${episode.title} · ${formatDuration(projectStore.episodeEffectiveDuration(episode))}`,
        x: columns.episode,
        y: 32 + index * 74,
        width: 165,
        episodeId: episode.id,
      });
    });
    return nodes;
  }

  function buildEdges(nodes: MapNode[]): MapEdge[] {
    const current = project;
    if (!current) return [];
    const edges: MapEdge[] = [];
    const known = new Set(nodes.map((node) => node.id));
    const menuNode = (menuId: string) => (known.has(`menu-${menuId}`) ? `menu-${menuId}` : null);
    const episodeNode = (episodeId: string) =>
      known.has(`ep-${episodeId}`) ? `ep-${episodeId}` : null;

    if (isContinuousAvchd) {
      if (known.has("continuous")) {
        current.episodes.forEach((episode) => {
          const target = episodeNode(episode.id);
          if (target) {
            edges.push({
              id: `continuous-${episode.id}`,
              from: "continuous",
              to: target,
              label: t("capítulos"),
              dashed: true,
            });
          }
        });
        for (let index = 1; index < current.episodes.length; index += 1) {
          const from = episodeNode(current.episodes[index - 1].id);
          const to = episodeNode(current.episodes[index].id);
          if (from && to) {
            edges.push({ id: `seq-${index}`, from, to, label: t("siguiente"), dashed: false });
          }
        }
      }
      return edges;
    }

    if (current.firstPlay && known.has("firstPlay")) {
      const target = menuNode(current.firstPlay.targetMenu);
      if (target) {
        edges.push({
          id: "first-target",
          from: "firstPlay",
          to: target,
          label: current.firstPlay.skippable ? t("omitir") : t("retorno"),
          dashed: true,
        });
      }
    }

    current.menus.forEach((menu) => {
      const source = menuNode(menu.id);
      if (!source) return;
      menu.buttons.forEach((button, index) => {
        const action = button.action;
        const label =
          action.kind === "jumpTitleChapter"
            ? `${t("capítulo")} ${action.chapter + 1}`
            : actionLabel(action.kind);
        if (action.kind === "gotoMenu") {
          const target = menuNode(action.menu);
          if (target && target !== source) {
            edges.push({ id: `${source}-b${index}`, from: source, to: target, label, dashed: false });
          }
        } else if (action.kind === "playEpisode") {
          const target = episodeNode(action.episode);
          if (target) {
            edges.push({ id: `${source}-b${index}`, from: source, to: target, label, dashed: false });
          }
        } else if (action.kind === "jumpTitleChapter") {
          const target = episodeNode(action.episode);
          if (target) {
            edges.push({ id: `${source}-b${index}`, from: source, to: target, label, dashed: false });
          }
        } else if (action.kind === "playAll") {
          current.episodes.forEach((episode) => {
            const target = episodeNode(episode.id);
            if (target) {
              edges.push({
                id: `${source}-b${index}-all-${episode.id}`,
                from: source,
                to: target,
                label: t("reproducir todo"),
                dashed: true,
              });
            }
          });
        } else if (action.kind === "playFirstPlay" && known.has("firstPlay")) {
          edges.push({ id: `${source}-b${index}-first`, from: source, to: "firstPlay", label, dashed: false });
        }
      });
      if (!isContinuousAvchd && rootMenu && menu.id !== rootMenu.id) {
        edges.push({
          id: `${source}-post`,
          from: source,
          to: `menu-${rootMenu.id}`,
          label: t("al terminar: volver"),
          dashed: true,
        });
      }
    });
    return edges;
  }

  const nodes = $derived(buildNodes());
  const edges = $derived(buildEdges(nodes));
  const mapWidth = $derived(
    isContinuousAvchd ? 520 : flowColumns(Boolean(project?.firstPlay)).width,
  );
  const canvasHeight = $derived(Math.max(240, ...nodes.map((node) => node.y + 100)));
  const selected = $derived(nodes.find((node) => node.id === selectedId) ?? nodes[0] ?? null);

  function nodeCenter(node: MapNode): { left: number; right: number; middle: number } {
    return {
      left: node.x,
      right: node.x + node.width,
      middle: node.y + 32,
    };
  }

  function edgePath(edge: MapEdge): string {
    const source = nodes.find((node) => node.id === edge.from);
    const target = nodes.find((node) => node.id === edge.to);
    if (!source || !target) return "";
    const from = nodeCenter(source);
    const to = nodeCenter(target);
    if (to.left <= from.right) {
      const drop = Math.max(from.middle, to.middle) + 38;
      return `M ${from.right} ${from.middle} C ${from.right + 50} ${drop}, ${to.left - 50} ${drop}, ${to.left} ${to.middle}`;
    }
    const control = Math.max(18, Math.min(80, (to.left - from.right) * 0.5));
    return `M ${from.right} ${from.middle} C ${from.right + control} ${from.middle}, ${to.left - control} ${to.middle}, ${to.left} ${to.middle}`;
  }

  function openSelected() {
    if (!selected) return;
    if (selected.menuId) projectStore.selectMenu(selected.menuId);
    else if (selected.episodeId) projectStore.selectEpisode(selected.episodeId);
  }

  function incoming(nodeId: string): MapEdge[] {
    return edges.filter((edge) => edge.to === nodeId);
  }

  function outgoing(nodeId: string): MapEdge[] {
    return edges.filter((edge) => edge.from === nodeId);
  }

  function nodeTitle(id: string): string {
    return nodes.find((node) => node.id === id)?.title ?? id;
  }
</script>

<section class="flex min-h-0 flex-1 flex-col bg-[#080d18]">
  <div class="flex flex-wrap items-center justify-between gap-2 border-b border-slate-800/70 px-3 py-2">
    <div class="flex items-center gap-2">
      <span class="text-[10px] font-semibold uppercase tracking-wider text-slate-500">{t("Mapa de navegación")}</span>
      <span class="rounded-full border border-cyan-500/30 bg-cyan-500/10 px-2 py-0.5 text-[10px] font-semibold text-cyan-200">
        {nodes.length} {t("nodos")} · {edges.length} {t("saltos")}
      </span>
      {#if isAvchd}
        <span class="rounded-full border border-slate-800 px-2 py-0.5 text-[10px] text-slate-400">
          {t(isContinuousAvchd ? "AVCHD continuo" : "Blu-ray/AVCHD HD")}
        </span>
      {/if}
    </div>
    <p class="text-[10px] text-slate-500">
      {t("INICIO → VMGM → VTSM → títulos · clic en un nodo para inspeccionarlo")}
    </p>
  </div>

  <div class="flex min-h-0 flex-1 flex-col gap-2 overflow-hidden p-3">
    <div class="min-h-0 flex-1 overflow-auto rounded-xl border border-slate-800 bg-slate-950/70">
      {#if nodes.length === 0}
        <div class="flex h-full items-center justify-center p-8 text-center text-xs text-slate-500">
          {t("Crea un proyecto DVD-Video para ver el flujo de la máquina virtual.")}
        </div>
      {:else}
        <div class="flex min-h-full items-center justify-center p-3">
          <div class="relative" style={`width: ${mapWidth}px; height: ${canvasHeight}px;`}>
            <svg class="pointer-events-none absolute inset-0 h-full w-full" viewBox={`0 0 ${mapWidth} ${canvasHeight}`}>
            <defs>
              <marker id="vm-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse">
                <path d="M 0 0 L 10 5 L 0 10 z" fill="#22d3ee" />
              </marker>
              <marker id="vm-arrow-dashed" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="5" markerHeight="5" orient="auto-start-reverse">
                <path d="M 0 0 L 10 5 L 0 10 z" fill="#94a3b8" />
              </marker>
            </defs>
            {#each edges as edge (edge.id)}
              <path
                d={edgePath(edge)}
                fill="none"
                stroke={edge.dashed ? "#64748b" : "#22d3ee"}
                stroke-width={edge.dashed ? 1.8 : 2.4}
                stroke-linecap="round"
                stroke-dasharray={edge.dashed ? "6 5" : undefined}
                marker-end={edge.dashed ? "url(#vm-arrow-dashed)" : "url(#vm-arrow)"}
                opacity={edge.dashed ? 0.7 : 0.85}
              />
            {/each}
          </svg>

          {#each nodes as node (node.id)}
            <button
              type="button"
              onclick={() => (selectedId = node.id)}
              ondblclick={openSelected}
              class={`absolute rounded-xl border px-2.5 py-2 text-left shadow-lg transition ${
                selected?.id === node.id
                  ? "border-cyan-400/80 bg-cyan-500/15 shadow-cyan-500/10"
                  : node.kind === "episode"
                    ? "border-slate-700 bg-slate-900/90 hover:border-cyan-500/50"
                    : node.kind === "firstPlay"
                      ? "border-amber-500/40 bg-amber-500/10 hover:border-amber-400/70"
                      : "border-slate-700 bg-slate-900/90 hover:border-cyan-500/50"
              }`}
              style={`left: ${node.x}px; top: ${node.y}px; width: ${node.width}px;`}
            >
              <span class="block truncate text-[10px] font-semibold text-slate-100">{node.title}</span>
              <span class="mt-0.5 block truncate text-[9px] text-slate-500">{node.subtitle}</span>
              <span class="mt-1 flex items-center gap-1 text-[9px] text-slate-600">
                <span class="rounded bg-slate-950/70 px-1 py-0.5">{incoming(node.id).length} ent.</span>
                <span class="rounded bg-slate-950/70 px-1 py-0.5">{outgoing(node.id).length} sal.</span>
                {#if node.kind === "episode"}
                  <span class="rounded bg-cyan-500/10 px-1 py-0.5 text-cyan-300">{t("título")}</span>
                {/if}
              </span>
            </button>
          {/each}
          </div>
        </div>
      {/if}
    </div>

    <div class="shrink-0 rounded-xl border border-slate-800 bg-slate-900/50 p-2.5">
      <div class="flex flex-wrap items-center gap-2">
        <span class="text-[9px] font-semibold uppercase tracking-wider text-slate-500">{t("Nodo")}</span>
        {#if selected}
          <span class="truncate text-[11px] font-semibold text-slate-100">{selected.title}</span>
          <span class="truncate text-[9px] text-slate-500">{selected.subtitle}</span>
          <span class="rounded bg-slate-950/70 px-1 py-0.5 text-[8px] text-slate-400">
            {incoming(selected.id).length} {t("ent.")} · {outgoing(selected.id).length} {t("sal.")}
          </span>
          <button
            type="button"
            onclick={openSelected}
            class="ml-auto rounded-md border border-cyan-500/40 bg-cyan-500/10 px-2 py-1 text-[9px] font-semibold text-cyan-200 transition hover:bg-cyan-500/20"
          >
            {t("Abrir")}
          </button>
        {:else}
          <span class="text-[10px] text-slate-500">{t("Sin nodo seleccionado.")}</span>
        {/if}
      </div>
      {#if selected}
        <div class="mt-1.5 flex gap-1 overflow-x-auto">
          {#each incoming(selected.id) as edge (edge.id)}
            <span class="shrink-0 rounded border border-slate-800 bg-slate-950/60 px-1.5 py-0.5 text-[8px] text-slate-400">
              ← {nodeTitle(edge.from)} · <span class="text-cyan-300/80">{edge.label}</span>
            </span>
          {/each}
          {#each outgoing(selected.id) as edge (edge.id)}
            <span class="shrink-0 rounded border border-slate-800 bg-slate-950/60 px-1.5 py-0.5 text-[8px] text-slate-400">
              → {nodeTitle(edge.to)} · <span class="text-cyan-300/80">{edge.label}</span>
            </span>
          {/each}
          {#if incoming(selected.id).length === 0 && outgoing(selected.id).length === 0}
            <span class="text-[8px] text-slate-600">{t("Sin conexiones.")}</span>
          {/if}
        </div>
      {/if}
      <div class="mt-1 flex flex-wrap items-center gap-2 text-[8px] text-slate-600">
        <span>{t("Líneas discontinuas: retornos automáticos o reproducir todo.")}</span>
        {#if projectStore.plan?.layerBreakSector !== null && projectStore.plan?.layerBreakSector !== undefined}
          <span class="rounded border border-amber-500/25 bg-amber-500/10 px-1 py-0.5 text-amber-200">
            {t("Corte de capa")} {projectStore.plan.layerBreakMode?.toUpperCase()} · {t("sector")} {projectStore.plan.layerBreakSector}
          </span>
        {/if}
      </div>
    </div>
  </div>
</section>
