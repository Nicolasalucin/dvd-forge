<script lang="ts">
  import Konva from "konva";
  import { onMount } from "svelte";
  import type { Menu_Serialize, NavigationGraph_Serialize } from "../api/bindings";
  import { drawBackground } from "./layers/BackgroundLayer";
  import { ButtonsLayer, type PreviewState } from "./layers/ButtonsLayer";
  import { drawNavigation, drawNavigationGraph } from "./layers/NavigationLayer";

  interface Props {
    menu: Menu_Serialize | null;
    backgroundColor: string;
    backgroundUrl?: string | null;
    thumbnails?: Record<string, string>;
    selectedButtonId?: string | null;
    focusButtonId?: string | null;
    previewState?: PreviewState;
    editable?: boolean;
    showSafeAreas?: boolean;
    showNavigation?: boolean;
    navigationGraph?: NavigationGraph_Serialize | null;
    onSelect?: (id: string) => void;
    onChange?: (
      id: string,
      rect: { x: number; y: number; w: number; h: number },
    ) => boolean;
    onActivate?: (id: string) => void;
  }

  let {
    menu,
    backgroundColor,
    backgroundUrl = null,
    thumbnails = {},
    selectedButtonId = null,
    focusButtonId = null,
    previewState = "normal",
    editable = true,
    showSafeAreas = true,
    showNavigation = true,
    navigationGraph = null,
    onSelect,
    onChange,
    onActivate,
  }: Props = $props();

  let wrapper: HTMLDivElement | undefined = $state();
  let container: HTMLDivElement | undefined = $state();
  let ready = $state(false);
  // Bumped when the Konva stage is created or destroyed so the layer effects
  // run again even though `stage` itself is a plain (non-reactive) handle.
  let stageRevision = $state(0);
  let stage: Konva.Stage | null = null;
  let backgroundLayer: Konva.Layer | null = null;
  let navigationLayer: Konva.Layer | null = null;
  let buttonsLayer: ButtonsLayer | null = null;

  onMount(() => {
    ready = true;
    if (!wrapper || !container) return;
    const observer = new ResizeObserver(() => resize());
    observer.observe(wrapper);
    return () => {
      observer.disconnect();
      destroyStage(false);
    };
  });

  // Lifecycle: only depends on whether a menu exists, not on its contents.
  $effect(() => {
    const activeMenu = menu;
    if (!ready || !container || !activeMenu) {
      if (!activeMenu) destroyStage();
      return;
    }
    if (stage) return;
    stage = new Konva.Stage({
      container,
      width: activeMenu.canvas.width,
      height: activeMenu.canvas.height,
    });
    backgroundLayer = new Konva.Layer({ listening: false });
    navigationLayer = new Konva.Layer({ listening: false });
    buttonsLayer = new ButtonsLayer();
    stage.add(backgroundLayer);
    stage.add(navigationLayer);
    stage.add(buttonsLayer.layer);
    stageRevision += 1;
  });

  // Background: expensive blur/dim/scale composition. It must not rerun when
  // the simulator only moves the focus between buttons.
  $effect(() => {
    stageRevision;
    if (!stage || !backgroundLayer || !menu) return;
    drawBackground(
      backgroundLayer,
      menu,
      backgroundColor,
      backgroundUrl,
      showSafeAreas,
    );
  });

  $effect(() => {
    stageRevision;
    if (!stage || !navigationLayer || !menu) return;
    if (!showNavigation) {
      navigationLayer.destroyChildren();
      navigationLayer.batchDraw();
      return;
    }
    if (navigationGraph) {
      drawNavigationGraph(
        navigationLayer,
        navigationGraph,
        focusButtonId ?? selectedButtonId,
      );
    } else {
      drawNavigation(navigationLayer, menu);
    }
  });

  $effect(() => {
    stageRevision;
    if (!stage || !buttonsLayer || !menu) return;
    buttonsLayer.sync(menu, {
      selectedId: selectedButtonId,
      focusId: focusButtonId,
      previewState,
      thumbnails,
      editable,
      callbacks: {
        onSelect,
        onChange,
        onActivate,
      },
    });
  });

  $effect(() => {
    stageRevision;
    if (!stage || !menu) return;
    // Track the native canvas size explicitly: a different menu may use
    // another resolution while reusing the same stage.
    void menu.canvas.width;
    void menu.canvas.height;
    resize();
  });

  function resize(): void {
    if (!stage || !wrapper || !container || !menu) return;
    const nativeWidth = menu.canvas.width;
    const nativeHeight = menu.canvas.height;
    // El marco tiene 6 px de borde y el contenedor exterior 8 px de padding.
    const availableWidth = wrapper.clientWidth - 28;
    const availableHeight = wrapper.clientHeight - 28;
    if (availableWidth <= 0 || availableHeight <= 0) return;
    const scale = Math.min(availableWidth / nativeWidth, availableHeight / nativeHeight);
    const width = Math.max(1, Math.floor(nativeWidth * scale));
    const height = Math.max(1, Math.floor(nativeHeight * scale));
    container.style.width = `${width + 12}px`;
    container.style.height = `${height + 12}px`;
    stage.size({ width, height });
    stage.scale({ x: scale, y: scale });
    stage.batchDraw();
  }

  function destroyStage(notify = true): void {
    if (!stage) return;
    stage.destroy();
    stage = null;
    backgroundLayer = null;
    navigationLayer = null;
    buttonsLayer = null;
    if (notify) stageRevision += 1;
  }
</script>

<div
  bind:this={wrapper}
  class="flex min-h-0 w-full flex-1 items-center justify-center rounded-2xl bg-slate-950/30 p-2"
>
  <div
    bind:this={container}
    class="relative flex-none overflow-hidden rounded-xl border-[6px] border-slate-800/90 bg-black shadow-[0_22px_55px_rgba(0,0,0,0.7)] ring-1 ring-black/70"
  ></div>
</div>
