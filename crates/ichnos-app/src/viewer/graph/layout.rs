//! Where the graph's cards go: fixed card sizes, one row per depth, each row centered.

use gpui_kit::Point;
use ichnos_model::model::{AgentBox, AgentGraph};

const PAD: f32 = 24.;
const CARD: f32 = 300.;
const MAIN_CARD: f32 = 340.;
const H_GAP: f32 = 56.;
const V_GAP: f32 = 90.;
pub(super) const TOOL_ROW: f32 = 21.;

/// Card positions: one row per depth, each row centered.
pub(super) struct Layout {
    /// Graph index -> top-left corner and size.
    pub(super) cards: Vec<(usize, Point<f32>, f32, f32)>,
    /// Parent index, child index.
    pub(super) edges: Vec<(usize, usize)>,
    pub(super) width: f32,
    pub(super) height: f32,
}

fn card_height(entry: &AgentBox) -> f32 {
    #[expect(clippy::cast_precision_loss, reason = "a card lists few tools")]
    let tools = entry.tools().len().max(1) as f32;
    30. + 22. + tools * TOOL_ROW + 8. + 28.
}

fn card_width(index: usize) -> f32 {
    if index == 0 { MAIN_CARD } else { CARD }
}

pub(super) fn layout(graph: &AgentGraph) -> Layout {
    let mut levels: Vec<Vec<usize>> = Vec::new();
    let mut edges = Vec::new();
    let mut stack = vec![(0_usize, 0_usize)];
    while let Some((index, depth)) = stack.pop() {
        if levels.len() <= depth {
            levels.resize(depth + 1, Vec::new());
        }
        levels[depth].push(index);
        if let Some(entry) = graph.get(index) {
            for &child in entry.children().iter().rev() {
                edges.push((index, child));
                stack.push((child, depth + 1));
            }
        }
    }
    let row_width = |level: &[usize]| -> f32 {
        let cards: f32 = level.iter().map(|&i| card_width(i)).sum();
        #[expect(clippy::cast_precision_loss, reason = "few cards per row")]
        let gaps = level.len().saturating_sub(1) as f32 * H_GAP;
        cards + gaps
    };
    let width = levels.iter().map(|l| row_width(l)).fold(0., f32::max) + PAD * 2.;
    let mut cards = Vec::new();
    let mut y = PAD;
    for level in &levels {
        let mut x = (width - row_width(level)) / 2.;
        let mut tallest: f32 = 0.;
        for &index in level {
            let Some(entry) = graph.get(index) else {
                continue;
            };
            let (w, h) = (card_width(index), card_height(entry));
            cards.push((index, Point { x, y }, w, h));
            x += w + H_GAP;
            tallest = tallest.max(h);
        }
        y += tallest + V_GAP;
    }
    Layout {
        cards,
        edges,
        width,
        height: y - V_GAP + PAD + 36.,
    }
}
