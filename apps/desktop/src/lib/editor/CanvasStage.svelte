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
      destroyStage();
    };
  });

  $effect(() => {
    if (!menu) {
      destroyStage();
      return;
    }
    if (!ready || !container) return;
    if (!stage) {
      stage = new Konva.Stage({
        container,
        width: menu.canvas.width,
        height: menu.canvas.height,
      });
      backgroundLayer = new Konva.Layer({ listening: false });
      navigationLayer = new Konva.Layer({ listening: false });
      buttonsLayer = new ButtonsLayer();
      stage.add(backgroundLayer);
      stage.add(navigationLayer);
      stage.add(buttonsLayer.layer);
    }
    if (!backgroundLayer || !navigationLayer || !buttonsLayer) return;

    drawBackground(
      backgroundLayer,
      menu,
      backgroundColor,
      backgroundUrl,
      showSafeAreas,
    );
    if (showNavigation) {
      if (navigationGraph) {
        drawNavigationGraph(navigationLayer, navigationGraph, focusButtonId ?? selectedButtonId);
      } else {
        drawNavigation(navigationLayer, menu);
      }
    } else {
      navigationLayer.destroyChildren();
      navigationLayer.batchDraw();
    }
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

  function destroyStage(): void {
    stage?.destroy();
    stage = null;
    backgroundLayer = null;
    navigationLayer = null;
    buttonsLayer = null;
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
