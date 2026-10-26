import Konva from "konva";
import type { Menu_Serialize, NavigationGraph_Serialize } from "../../api/bindings";
import { normalizeRect, type PixelRect } from "../exportMenu";

interface Node {
  id: string;
  rect: PixelRect;
  cx: number;
  cy: number;
  enabled: boolean;
  override: {
    up: string | null;
    down: string | null;
    left: string | null;
    right: string | null;
  };
}

const DIRECTIONS = [
  { key: "right", dx: 1, dy: 0 },
  { key: "left", dx: -1, dy: 0 },
  { key: "down", dx: 0, dy: 1 },
  { key: "up", dx: 0, dy: -1 },
] as const;

type Direction = (typeof DIRECTIONS)[number];

/** Dibuja la matriz de navegación efectiva del editor. La simulación usa
 * `drawNavigationGraph` con el grafo calculado por Rust. */
export function drawNavigation(layer: Konva.Layer, menu: Menu_Serialize): void {
  layer.destroyChildren();
  if (menu.buttons.length < 2) return;
  const nodes: Node[] = menu.buttons.map((button) => {
    const rect = normalizeRect(button.rect, menu.canvas.width, menu.canvas.height);
    return {
      id: button.id,
      rect,
      cx: (rect.x0 + rect.x1) / 2,
      cy: (rect.y0 + rect.y1) / 2,
      enabled: button.enabled,
      override: {
        up: button.navOverride?.up ?? null,
        down: button.navOverride?.down ?? null,
        left: button.navOverride?.left ?? null,
        right: button.navOverride?.right ?? null,
      },
    };
  });
  const enabled = nodes.filter((node) => node.enabled);

  for (const node of enabled) {
    for (const direction of DIRECTIONS) {
      const manual = node.override[direction.key];
      const target = manual
        ? (enabled.find((candidate) => candidate.id === manual) ??
          bestTarget(node, enabled, direction))
        : bestTarget(node, enabled, direction);
      if (!target || target.id === node.id) continue;
      layer.add(
        new Konva.Arrow({
          points: [node.cx, node.cy, target.cx, target.cy],
          stroke: "#38bdf8",
          fill: "#38bdf8",
          strokeWidth: 2,
          pointerLength: 8,
          pointerWidth: 8,
          dash: [7, 7],
          opacity: manual ? 0.72 : 0.42,
          listening: false,
        }),
      );
    }
  }
  layer.batchDraw();
}

/** Pinta exactamente las aristas que usará el simulador, incluido el foco. */
export function drawNavigationGraph(
  layer: Konva.Layer,
  graph: NavigationGraph_Serialize,
  focus: string | null,
): void {
  layer.destroyChildren();
  const centers = new Map(
    graph.nodes.map((node) => [node.button, { x: node.center.x ?? 0, y: node.center.y ?? 0 }]),
  );
  for (const edge of graph.edges) {
    const from = centers.get(edge.from);
    const to = centers.get(edge.to);
    if (!from || !to || edge.from === edge.to) continue;
    const manual = edge.source === "manual";
    const active = focus === edge.from;
    layer.add(
      new Konva.Arrow({
        points: [from.x, from.y, to.x, to.y],
        stroke: active ? "#fbbf24" : "#38bdf8",
        fill: active ? "#fbbf24" : "#38bdf8",
        strokeWidth: active ? 3 : 2,
        pointerLength: 8,
        pointerWidth: 8,
        dash: [7, 7],
        opacity: active ? 0.9 : manual ? 0.55 : 0.3,
        listening: false,
      }),
    );
  }
  layer.batchDraw();
}

function bestTarget(current: Node, nodes: Node[], direction: Direction): Node | null {
  const candidates: Array<{ node: Node; forward: number; lateral: number; overlap: number }> = [];
  for (const candidate of nodes) {
    if (candidate.id === current.id) continue;
    const dx = candidate.cx - current.cx;
    const dy = candidate.cy - current.cy;
    const forward = dx * direction.dx + dy * direction.dy;
    if (forward <= 0.5) continue;
    const lateral = Math.abs(dx * direction.dy - dy * direction.dx);
    const overlap =
      direction.dx !== 0
        ? Math.max(
            0,
            Math.min(current.rect.y1, candidate.rect.y1) -
              Math.max(current.rect.y0, candidate.rect.y0),
          )
        : Math.max(
            0,
            Math.min(current.rect.x1, candidate.rect.x1) -
              Math.max(current.rect.x0, candidate.rect.x0),
          );
    candidates.push({ node: candidate, forward, lateral, overlap });
  }
  if (candidates.length === 0) return null;

  const inCone = candidates.filter(
    (candidate) => candidate.lateral <= candidate.forward * 2 + 32,
  );
  const pool = inCone.length > 0 ? inCone : candidates;
  pool.sort(
    (a, b) =>
      a.forward + 1.5 * a.lateral - 0.75 * a.overlap -
        (b.forward + 1.5 * b.lateral - 0.75 * b.overlap) ||
      b.overlap - a.overlap ||
      a.forward - b.forward ||
      a.lateral - b.lateral ||
      a.node.id.localeCompare(b.node.id),
  );
  return pool[0]?.node ?? null;
}
