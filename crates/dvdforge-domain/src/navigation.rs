use crate::diagnostic::Diagnostic;
use crate::geometry::{normalize_rect, CanvasSpec, PixelRect, Point};
use crate::ids::{ButtonId, MenuId};
use crate::menu::{ButtonAction, MenuButton, NavOverride};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum NavDirection {
    Up,
    Down,
    Left,
    Right,
}

impl NavDirection {
    pub const ALL: [Self; 4] = [Self::Up, Self::Down, Self::Left, Self::Right];

    pub const fn opposite(self) -> Self {
        match self {
            Self::Up => Self::Down,
            Self::Down => Self::Up,
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Down => "down",
            Self::Left => "left",
            Self::Right => "right",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum EdgeSource {
    Manual,
    Geometric,
    SelfLoop,
    Forced,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct NavPolicy {
    pub gap_px: u16,
    /// Weight of the perpendicular distance in the score.
    pub axis_bias: f64,
    pub allow_self_loop: bool,
    pub wrap_edges: bool,
    pub require_all_directions: bool,
    /// Slope of the directional cone. 2.0 means roughly +/-63 degrees.
    pub cone_slope: f64,
    /// Extra cone width in pixels, useful for small buttons.
    pub cone_margin: f64,
}

impl Default for NavPolicy {
    fn default() -> Self {
        Self {
            gap_px: 2,
            axis_bias: 1.5,
            allow_self_loop: true,
            wrap_edges: false,
            require_all_directions: true,
            cone_slope: 2.0,
            cone_margin: 32.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct NavNode {
    pub button: ButtonId,
    pub rect: PixelRect,
    pub action: ButtonAction,
    pub center: Point,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct NavEdge {
    pub from: ButtonId,
    pub direction: NavDirection,
    pub to: ButtonId,
    pub source: EdgeSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct NavigationGraph {
    pub menu_id: MenuId,
    pub nodes: Vec<NavNode>,
    pub edges: Vec<NavEdge>,
    pub default_focus: Option<ButtonId>,
    pub policy: NavPolicy,
    pub route_hash: String,
    pub warnings: Vec<Diagnostic>,
}

impl NavigationGraph {
    pub fn edge(&self, from: ButtonId, direction: NavDirection) -> Option<&NavEdge> {
        self.edges
            .iter()
            .find(|edge| edge.from == from && edge.direction == direction)
    }

    pub fn target(&self, from: ButtonId, direction: NavDirection) -> Option<ButtonId> {
        self.edge(from, direction).map(|edge| edge.to)
    }

    pub fn has_errors(&self) -> bool {
        self.warnings.iter().any(Diagnostic::is_error)
    }
}

/// Computes deterministic directional navigation from button geometry.
///
/// The algorithm favors buttons that are in the same row/column and penalizes
/// large perpendicular offsets. Manual overrides always win, but invalid
/// overrides are reported and fall back to geometry.
pub fn resolve_navigation(
    menu_id: MenuId,
    canvas: &CanvasSpec,
    buttons: &[MenuButton],
    policy: &NavPolicy,
    default_focus: Option<ButtonId>,
) -> NavigationGraph {
    let mut warnings = Vec::new();
    let mut nodes = Vec::new();

    for button in buttons.iter().filter(|button| button.enabled) {
        match normalize_rect(button.rect, *canvas, 8) {
            Ok(rect) => nodes.push(NavNode {
                button: button.id,
                rect,
                action: button.action.clone(),
                center: rect.center(),
            }),
            Err(diagnostic) => {
                warnings.push(diagnostic.with_entity(format!("button:{}", button.id)));
            }
        }
    }

    nodes.sort_by(|a, b| {
        a.rect
            .y0
            .cmp(&b.rect.y0)
            .then_with(|| a.rect.x0.cmp(&b.rect.x0))
            .then_with(|| a.button.cmp(&b.button))
    });

    if nodes.is_empty() {
        warnings.push(
            Diagnostic::error("E_MENU_NO_BUTTONS", "El menú no tiene botones habilitados.")
                .with_entity(format!("menu:{menu_id}")),
        );
        return NavigationGraph {
            menu_id,
            nodes,
            edges: Vec::new(),
            default_focus: None,
            policy: policy.clone(),
            route_hash: route_hash(&[]),
            warnings,
        };
    }

    let resolved_default = match default_focus {
        Some(id) if nodes.iter().any(|node| node.button == id) => Some(id),
        Some(id) => {
            warnings.push(
                Diagnostic::warning(
                    "W_DEFAULT_FOCUS_INVALID",
                    "El foco inicial no existe o está deshabilitado; se usa el primer botón.",
                )
                .with_entity(format!("button:{id}")),
            );
            Some(nodes[0].button)
        }
        None => Some(nodes[0].button),
    };

    let mut edges = Vec::with_capacity(nodes.len() * 4);
    for current in &nodes {
        let button = buttons.iter().find(|button| button.id == current.button);
        for direction in NavDirection::ALL {
            if let Some(target) =
                button.and_then(|button| override_target(&button.nav_override, direction))
            {
                if nodes.iter().any(|node| node.button == target) {
                    edges.push(NavEdge {
                        from: current.button,
                        direction,
                        to: target,
                        source: EdgeSource::Manual,
                    });
                    continue;
                }
                warnings.push(
                    Diagnostic::error(
                        "E_NAV_OVERRIDE_INVALID",
                        format!(
                            "El override {} del botón apunta a un botón inexistente o deshabilitado.",
                            direction.label()
                        ),
                    )
                    .with_entity(format!("button:{}", current.button)),
                );
            }

            if let Some(target) = best_candidate(current, &nodes, direction, policy) {
                edges.push(NavEdge {
                    from: current.button,
                    direction,
                    to: target.button,
                    source: EdgeSource::Geometric,
                });
            } else if policy.wrap_edges {
                if let Some(target) = wrap_candidate(current, &nodes, direction) {
                    edges.push(NavEdge {
                        from: current.button,
                        direction,
                        to: target.button,
                        source: EdgeSource::Geometric,
                    });
                } else if policy.allow_self_loop {
                    edges.push(self_loop(current.button, direction));
                } else {
                    warnings.push(no_neighbor(current.button, direction));
                }
            } else if policy.allow_self_loop {
                edges.push(self_loop(current.button, direction));
            } else {
                warnings.push(no_neighbor(current.button, direction));
            }
        }
    }

    normalize_symmetry(&nodes, &mut edges, &mut warnings);
    if let Some(focus) = resolved_default {
        mark_unreachable(&nodes, &edges, focus, &mut warnings);
    }

    let route_hash = route_hash(&edges);
    NavigationGraph {
        menu_id,
        nodes,
        edges,
        default_focus: resolved_default,
        policy: policy.clone(),
        route_hash,
        warnings,
    }
}

fn route_hash(edges: &[NavEdge]) -> String {
    let mut entries: Vec<String> = edges
        .iter()
        .map(|edge| {
            format!(
                "{}:{}:{}:{:?}",
                edge.from, edge.direction as u8, edge.to, edge.source
            )
        })
        .collect();
    entries.sort();
    let mut hasher = sha2::Sha256::new();
    for entry in entries {
        hasher.update(entry.as_bytes());
        hasher.update([0]);
    }
    use sha2::Digest;
    hex::encode(hasher.finalize())
}

fn normalize_symmetry(nodes: &[NavNode], edges: &mut Vec<NavEdge>, warnings: &mut Vec<Diagnostic>) {
    let ids: HashSet<ButtonId> = nodes.iter().map(|node| node.button).collect();
    let snapshot = edges.clone();
    let mut additions = Vec::new();
    for edge in snapshot {
        if edge.from == edge.to
            || !matches!(edge.direction, NavDirection::Right | NavDirection::Down)
        {
            continue;
        }
        let reverse_index = edges.iter().position(|candidate| {
            candidate.from == edge.to && candidate.direction == edge.direction.opposite()
        });
        match reverse_index {
            Some(index) if edges[index].to == edge.from => {}
            Some(index)
                if edges[index].source == EdgeSource::Manual
                    && edge.source == EdgeSource::Manual =>
            {
                warnings.push(
                    Diagnostic::error(
                        "E_NAV_ROUTE_CONFLICT",
                        "Dos overrides manuales apuntan en direcciones opuestas a botones distintos.",
                    )
                    .with_entity(format!("button:{}", edge.from)),
                );
            }
            Some(index) if edges[index].source == EdgeSource::Manual => {
                warnings.push(
                    Diagnostic::warning(
                        "W_NAV_ROUTE_APPROX",
                        "El override manual impide una simetría perfecta con este botón.",
                    )
                    .with_entity(format!("button:{}", edge.from)),
                );
            }
            Some(index) => {
                edges[index].to = edge.from;
                edges[index].source = EdgeSource::Forced;
                warnings.push(
                    Diagnostic::warning(
                        "W_NAV_ROUTE_APPROX",
                        "Se ajustó el vecino inverso para mantener la simetría del D-Pad.",
                    )
                    .with_entity(format!("button:{}", edge.from)),
                );
            }
            None => additions.push(NavEdge {
                from: edge.to,
                direction: edge.direction.opposite(),
                to: edge.from,
                source: EdgeSource::Forced,
            }),
        }
    }
    for addition in additions {
        let from = addition.from;
        if ids.contains(&from)
            && !edges
                .iter()
                .any(|edge| edge.from == from && edge.direction == addition.direction)
        {
            edges.push(addition);
            warnings.push(
                Diagnostic::warning(
                    "W_NAV_ROUTE_APPROX",
                    "Se añadió un vecino forzado para mantener la simetría del D-Pad.",
                )
                .with_entity(format!("button:{from}")),
            );
        }
    }
}

fn self_loop(button: ButtonId, direction: NavDirection) -> NavEdge {
    NavEdge {
        from: button,
        direction,
        to: button,
        source: EdgeSource::SelfLoop,
    }
}

fn no_neighbor(button: ButtonId, direction: NavDirection) -> Diagnostic {
    Diagnostic::warning(
        "W_NAV_NO_NEIGHBOR",
        format!(
            "El botón no tiene vecino {} y los auto-bucles están desactivados.",
            direction.label()
        ),
    )
    .with_entity(format!("button:{button}"))
}

fn override_target(overrides: &NavOverride, direction: NavDirection) -> Option<ButtonId> {
    match direction {
        NavDirection::Up => overrides.up,
        NavDirection::Down => overrides.down,
        NavDirection::Left => overrides.left,
        NavDirection::Right => overrides.right,
    }
}

#[derive(Debug, Clone, Copy)]
struct Metrics {
    forward: f64,
    lateral: f64,
    overlap: f64,
}

impl Metrics {
    fn score(self, policy: &NavPolicy) -> f64 {
        self.forward + policy.axis_bias * self.lateral - 0.75 * self.overlap
    }
}

fn metrics(current: &NavNode, other: &NavNode, direction: NavDirection) -> Option<Metrics> {
    let dx = other.center.x - current.center.x;
    let dy = other.center.y - current.center.y;
    let (forward, lateral, overlap) = match direction {
        NavDirection::Right => (dx, dy.abs(), current.rect.overlap_y(other.rect) as f64),
        NavDirection::Left => (-dx, dy.abs(), current.rect.overlap_y(other.rect) as f64),
        NavDirection::Down => (dy, dx.abs(), current.rect.overlap_x(other.rect) as f64),
        NavDirection::Up => (-dy, dx.abs(), current.rect.overlap_x(other.rect) as f64),
    };
    if forward <= 0.5 {
        return None;
    }
    Some(Metrics {
        forward,
        lateral,
        overlap,
    })
}

fn best_candidate<'a>(
    current: &NavNode,
    nodes: &'a [NavNode],
    direction: NavDirection,
    policy: &NavPolicy,
) -> Option<&'a NavNode> {
    let mut candidates: Vec<(&NavNode, Metrics)> = nodes
        .iter()
        .filter(|node| node.button != current.button)
        .filter_map(|node| metrics(current, node, direction).map(|metrics| (node, metrics)))
        .collect();

    if candidates.is_empty() {
        return None;
    }

    let in_cone: Vec<(&NavNode, Metrics)> = candidates
        .iter()
        .copied()
        .filter(|(_, metrics)| {
            metrics.lateral <= metrics.forward * policy.cone_slope + policy.cone_margin
        })
        .collect();

    if !in_cone.is_empty() {
        candidates = in_cone;
    }

    candidates
        .into_iter()
        .min_by(|a, b| compare_candidates(a, b, policy))
        .map(|(node, _)| node)
}

fn compare_candidates(
    a: &(&NavNode, Metrics),
    b: &(&NavNode, Metrics),
    policy: &NavPolicy,
) -> Ordering {
    a.1.score(policy)
        .partial_cmp(&b.1.score(policy))
        .unwrap_or(Ordering::Equal)
        .then_with(|| {
            b.1.overlap
                .partial_cmp(&a.1.overlap)
                .unwrap_or(Ordering::Equal)
        })
        .then_with(|| {
            a.1.forward
                .partial_cmp(&b.1.forward)
                .unwrap_or(Ordering::Equal)
        })
        .then_with(|| {
            a.1.lateral
                .partial_cmp(&b.1.lateral)
                .unwrap_or(Ordering::Equal)
        })
        .then_with(|| a.0.button.cmp(&b.0.button))
}

fn wrap_candidate<'a>(
    current: &NavNode,
    nodes: &'a [NavNode],
    direction: NavDirection,
) -> Option<&'a NavNode> {
    let others: Vec<&NavNode> = nodes
        .iter()
        .filter(|node| node.button != current.button)
        .collect();
    if others.is_empty() {
        return None;
    }

    others.into_iter().min_by(|a, b| {
        let key_a = wrap_key(a, direction);
        let key_b = wrap_key(b, direction);
        key_a
            .partial_cmp(&key_b)
            .unwrap_or(Ordering::Equal)
            .then_with(|| {
                let lateral_a = wrap_lateral(a, current, direction);
                let lateral_b = wrap_lateral(b, current, direction);
                lateral_a.partial_cmp(&lateral_b).unwrap_or(Ordering::Equal)
            })
            .then_with(|| a.button.cmp(&b.button))
    })
}

fn wrap_key(node: &NavNode, direction: NavDirection) -> f64 {
    match direction {
        NavDirection::Right => node.center.x,
        NavDirection::Left => -node.center.x,
        NavDirection::Down => node.center.y,
        NavDirection::Up => -node.center.y,
    }
}

fn wrap_lateral(node: &NavNode, current: &NavNode, direction: NavDirection) -> f64 {
    match direction {
        NavDirection::Right | NavDirection::Left => (node.center.y - current.center.y).abs(),
        NavDirection::Down | NavDirection::Up => (node.center.x - current.center.x).abs(),
    }
}

fn mark_unreachable(
    nodes: &[NavNode],
    edges: &[NavEdge],
    focus: ButtonId,
    warnings: &mut Vec<Diagnostic>,
) {
    let mut adjacency: HashMap<ButtonId, Vec<ButtonId>> = HashMap::new();
    for edge in edges {
        adjacency.entry(edge.from).or_default().push(edge.to);
    }

    let mut visited = HashSet::new();
    let mut stack = vec![focus];
    while let Some(button) = stack.pop() {
        if !visited.insert(button) {
            continue;
        }
        if let Some(neighbors) = adjacency.get(&button) {
            stack.extend(neighbors.iter().copied());
        }
    }

    for node in nodes {
        if !visited.contains(&node.button) {
            warnings.push(
                Diagnostic::warning(
                    "W_NAV_UNREACHABLE",
                    "El botón no es alcanzable desde el foco inicial.",
                )
                .with_entity(format!("button:{}", node.button)),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{AspectRatio, LogicalRect, TvStandard};
    use crate::ids::ButtonId;

    fn canvas() -> CanvasSpec {
        CanvasSpec::new(TvStandard::Ntsc, AspectRatio::FourThree)
    }

    fn button(id: u128, x: f64, y: f64) -> MenuButton {
        MenuButton {
            id: ButtonId::from_u128(id),
            name: format!("b{id}"),
            rect: LogicalRect::new(x, y, 192.0, 72.0),
            z: 0,
            action: ButtonAction::Noop,
            thumbnail: None,
            label: None,
            style_kind: crate::menu::ButtonStyleKind::ClassicFrame,
            font_family: "sans-serif".to_string(),
            font_size: 22,
            font_weight: "normal".to_string(),
            text_color: "#FFFFFF".to_string(),
            shadow_color: "#000000".to_string(),
            fill_color: "#0F172A".to_string(),
            shape_mask: crate::menu::ButtonShapeMask::Rectangle,
            motion_preview: None,
            nav_override: NavOverride::default(),
            enabled: true,
        }
    }

    fn grid() -> Vec<MenuButton> {
        vec![
            button(1, 60.0, 132.0),
            button(2, 276.0, 132.0),
            button(3, 492.0, 132.0),
            button(4, 60.0, 252.0),
            button(5, 276.0, 252.0),
            button(6, 492.0, 252.0),
        ]
    }

    fn target(graph: &NavigationGraph, from: u128, direction: NavDirection) -> u128 {
        graph
            .target(ButtonId::from_u128(from), direction)
            .expect("edge exists")
            .as_uuid()
            .as_u128()
    }

    #[test]
    fn randomized_rectangles_are_deterministic_and_well_formed() {
        let mut state = 0x9E37_79B9_u64;
        let buttons: Vec<MenuButton> = (0..24)
            .map(|index| {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                let x = ((state >> 16) % 20) as f64 * 32.0;
                let y = ((state >> 32) % 10) as f64 * 32.0;
                let mut current = button(index + 1, x, y);
                current.rect = LogicalRect::new(x, y, 120.0, 60.0);
                current
            })
            .collect();
        let first = resolve_navigation(
            MenuId::from_u128(7),
            &canvas(),
            &buttons,
            &NavPolicy::default(),
            None,
        );
        for _ in 0..10 {
            let next = resolve_navigation(
                MenuId::from_u128(7),
                &canvas(),
                &buttons,
                &NavPolicy::default(),
                None,
            );
            assert_eq!(first.route_hash, next.route_hash);
        }
        let ids: std::collections::HashSet<_> = buttons.iter().map(|button| button.id).collect();
        assert!(first
            .edges
            .iter()
            .all(|edge| ids.contains(&edge.from) && ids.contains(&edge.to)));
    }

    #[test]
    fn grid_navigation_matches_geometry() {
        let graph = resolve_navigation(
            MenuId::from_u128(99),
            &canvas(),
            &grid(),
            &NavPolicy::default(),
            Some(ButtonId::from_u128(1)),
        );

        assert_eq!(target(&graph, 1, NavDirection::Right), 2);
        assert_eq!(target(&graph, 1, NavDirection::Down), 4);
        assert_eq!(target(&graph, 1, NavDirection::Left), 1);
        assert_eq!(target(&graph, 1, NavDirection::Up), 1);

        assert_eq!(target(&graph, 2, NavDirection::Right), 3);
        assert_eq!(target(&graph, 2, NavDirection::Down), 5);
        assert_eq!(target(&graph, 3, NavDirection::Right), 3);
        assert_eq!(target(&graph, 3, NavDirection::Down), 6);

        assert_eq!(target(&graph, 4, NavDirection::Up), 1);
        assert_eq!(target(&graph, 4, NavDirection::Right), 5);
        assert_eq!(target(&graph, 5, NavDirection::Left), 4);
        assert_eq!(target(&graph, 5, NavDirection::Right), 6);
        assert_eq!(target(&graph, 6, NavDirection::Left), 5);
        assert_eq!(target(&graph, 6, NavDirection::Up), 3);
    }

    #[test]
    fn manual_override_wins() {
        let mut buttons = grid();
        buttons[0].nav_override.right = Some(ButtonId::from_u128(3));
        let graph = resolve_navigation(
            MenuId::from_u128(99),
            &canvas(),
            &buttons,
            &NavPolicy::default(),
            None,
        );
        let edge = graph
            .edge(ButtonId::from_u128(1), NavDirection::Right)
            .expect("edge");
        assert_eq!(edge.to, ButtonId::from_u128(3));
        assert_eq!(edge.source, EdgeSource::Manual);
    }

    #[test]
    fn invalid_override_falls_back_to_geometry() {
        let mut buttons = grid();
        buttons[0].nav_override.right = Some(ButtonId::from_u128(999));
        let graph = resolve_navigation(
            MenuId::from_u128(99),
            &canvas(),
            &buttons,
            &NavPolicy::default(),
            None,
        );
        assert_eq!(target(&graph, 1, NavDirection::Right), 2);
        assert!(graph
            .warnings
            .iter()
            .any(|d| d.code == "E_NAV_OVERRIDE_INVALID"));
    }

    #[test]
    fn single_button_self_loops() {
        let graph = resolve_navigation(
            MenuId::from_u128(99),
            &canvas(),
            &[button(1, 60.0, 132.0)],
            &NavPolicy::default(),
            None,
        );
        for direction in NavDirection::ALL {
            let edge = graph.edge(ButtonId::from_u128(1), direction).expect("edge");
            assert_eq!(edge.to, ButtonId::from_u128(1));
            assert_eq!(edge.source, EdgeSource::SelfLoop);
        }
    }

    #[test]
    fn route_hash_is_stable_and_symmetric() {
        let buttons = grid();
        let first = resolve_navigation(
            MenuId::from_u128(99),
            &canvas(),
            &buttons,
            &NavPolicy::default(),
            None,
        );
        for _ in 0..50 {
            let next = resolve_navigation(
                MenuId::from_u128(99),
                &canvas(),
                &buttons,
                &NavPolicy::default(),
                None,
            );
            assert_eq!(first.route_hash, next.route_hash);
        }
        for edge in &first.edges {
            if edge.from == edge.to {
                continue;
            }
            let reverse = first
                .edges
                .iter()
                .find(|candidate| {
                    candidate.from == edge.to && candidate.direction == edge.direction.opposite()
                })
                .expect("reverse edge");
            assert_eq!(reverse.to, edge.from);
        }
    }

    #[test]
    fn empty_menu_reports_error() {
        let graph = resolve_navigation(
            MenuId::from_u128(99),
            &canvas(),
            &[],
            &NavPolicy::default(),
            None,
        );
        assert!(graph.warnings.iter().any(|d| d.code == "E_MENU_NO_BUTTONS"));
    }
}
