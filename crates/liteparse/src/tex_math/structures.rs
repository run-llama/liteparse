//! Two-dimensional math from rules: fractions, overlines and square roots.
//!
//! A rule is a thin horizontal rect, line, or stretched image (some PDFs draw
//! TeX's `\vrule` as a 1x1 image). Rules are taken narrowest first, so an
//! inner fraction is folded before the one holding it: its glyphs become one
//! stand-in glyph whose text is the finished LaTeX.

use std::collections::HashSet;

use super::{G, PRE, Role, attach_accents, build, is_alpha, is_operator, max_size, median};

/// A thin horizontal rule. `y` is y-up page space (`page height - y`).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Rule {
    pub(crate) x0: f32,
    pub(crate) x1: f32,
    pub(crate) y: f32,
    pub(crate) w: f32,
}

impl Rule {
    /// A rule from a viewport-space box (y down), when the box is thin and
    /// wide enough to be one.
    pub(crate) fn from_box(x0: f32, x1: f32, top: f32, bottom: f32, page_h: f32) -> Option<Rule> {
        (bottom - top <= 1.2 && x1 - x0 >= 1.5).then(|| Rule {
            x0,
            x1,
            y: page_h - (top + bottom) / 2.0,
            w: x1 - x0,
        })
    }
}

fn tex_of(all: &mut [G], gs: &[usize]) -> String {
    let mut sorted = gs.to_vec();
    sorted.sort_by(|&a, &b| all[a].x0.total_cmp(&all[b].x0));
    let kept = attach_accents(all, &sorted);
    if kept.is_empty() {
        return String::new();
    }
    let big = max_size(all, &kept);
    let base = median(
        kept.iter()
            .filter(|&&g| all[g].size >= 0.92 * big)
            .map(|&g| all[g].base)
            .collect(),
    );
    build(all, &kept, base, big)
}

#[allow(clippy::too_many_arguments)]
/// Family of a folded structure's stand-in glyph.
pub(crate) const STAND_IN_FAM: &str = "SYN";

fn stand_in(all: &[G], tex: String, parts: &[usize], x0: f32, x1: f32, base: f32, size: f32) -> G {
    G {
        t: format!("{PRE}{tex}"),
        role: Some(Role::Mi),
        fam: STAND_IN_FAM.to_string(),
        size,
        base,
        x0,
        x1,
        top: parts.iter().map(|&g| all[g].top).fold(f32::MAX, f32::min),
        bottom: parts
            .iter()
            .map(|&g| all[g].bottom)
            .fold(f32::MIN, f32::max),
        loose: (
            parts
                .iter()
                .map(|&g| all[g].loose.0)
                .fold(f32::MAX, f32::min),
            parts
                .iter()
                .map(|&g| all[g].loose.1)
                .fold(f32::MIN, f32::max),
        ),
        pre: String::new(),
        post: String::new(),
        src: parts
            .iter()
            .flat_map(|&g| all[g].src.iter().copied())
            .collect(),
    }
}

/// The glyphs stacked right above (or below) a rule, within its width: the
/// baseline nearest the rule and the scripts riding on it. Glyphs much
/// larger than the nearest ones are a neighbouring text line, not the
/// fraction (a text-style fraction is set at script size).
fn stack(all: &[G], live: &[usize], r: &Rule, h: f32, above: bool) -> Vec<usize> {
    let near = live
        .iter()
        .copied()
        .filter(|&g| r.x0 - 0.8 <= all[g].cx() && all[g].cx() <= r.x1 + 0.8);
    let (cand, s, nb) = if above {
        let cand: Vec<usize> = near
            .filter(|&g| (h - all[g].bottom) >= r.y - 0.6 && all[g].base - r.y < 1.6 * all[g].size)
            .collect();
        let Some(g0) = cand
            .iter()
            .copied()
            .reduce(|a, b| if all[b].base < all[a].base { b } else { a })
        else {
            return Vec::new();
        };
        let s = cand
            .iter()
            .filter(|&&g| all[g].base <= all[g0].base + 0.4 * all[g].size)
            .map(|&g| all[g].size)
            .fold(f32::MIN, f32::max);
        let nb = cand
            .iter()
            .filter(|&&g| all[g].size >= 0.85 * s)
            .map(|&g| all[g].base)
            .fold(f32::MAX, f32::min);
        (cand, s, nb)
    } else {
        let cand: Vec<usize> = near
            .filter(|&g| {
                (h - all[g].top) <= r.y + 0.6 && r.y - (h - all[g].top) < 0.9 * all[g].size
            })
            .collect();
        let Some(g0) = cand.iter().copied().reduce(|a, b| {
            if (h - all[b].top) > (h - all[a].top) {
                b
            } else {
                a
            }
        }) else {
            return Vec::new();
        };
        let s = cand
            .iter()
            .filter(|&&g| (h - all[g].top) >= (h - all[g0].top) - 0.4 * all[g].size)
            .map(|&g| all[g].size)
            .fold(f32::MIN, f32::max);
        let nb = cand
            .iter()
            .filter(|&&g| all[g].size >= 0.85 * s)
            .map(|&g| all[g].base)
            .fold(f32::MIN, f32::max);
        (cand, s, nb)
    };
    cand.into_iter()
        .filter(|&g| {
            all[g].size <= 1.15 * s && nb - 0.45 * s <= all[g].base && all[g].base <= nb + 0.6 * s
        })
        .collect()
}

/// Whether glyphs hold a word of running text (3+ text-font letters that
/// aren't an operator name): table cells over and under a rule, not a
/// fraction.
fn wordy(all: &[G], gs: &[usize]) -> bool {
    let mut sorted = gs.to_vec();
    sorted.sort_by(|&a, &b| all[a].x0.total_cmp(&all[b].x0));
    let mut run = String::new();
    let mut run_len = 0usize;
    for g in sorted.iter().map(Some).chain(std::iter::once(None)) {
        if let Some(&g) = g
            && all[g].role.is_none()
            && is_alpha(&all[g].t)
        {
            run.push_str(&all[g].t);
            run_len += 1;
            continue;
        }
        if run_len >= 3 && !is_operator(&run) {
            return true;
        }
        run.clear();
        run_len = 0;
    }
    false
}

fn width(all: &[G], gs: &[usize]) -> (f32, f32) {
    (
        gs.iter().map(|&g| all[g].x0).fold(f32::MAX, f32::min),
        gs.iter().map(|&g| all[g].x1).fold(f32::MIN, f32::max),
    )
}

fn is_radical(t: &str) -> bool {
    matches!(t, "\0SQRT" | "\0\\surd " | "√" | "\0\\surd")
}

/// Replace `parts` in `live` with a new stand-in glyph.
fn replace(all: &mut Vec<G>, live: Vec<usize>, parts: &[usize], sy: G) -> Vec<usize> {
    let gone: HashSet<usize> = parts.iter().copied().collect();
    all.push(sy);
    let new = all.len() - 1;
    live.into_iter()
        .filter(|g| !gone.contains(g))
        .chain(std::iter::once(new))
        .collect()
}

/// Fold fractions, square roots and overlines into stand-in glyphs. Returns
/// the live glyph list; stand-ins are appended to `all`.
pub(super) fn fold(all: &mut Vec<G>, mut live: Vec<usize>, rules: &[Rule], h: f32) -> Vec<usize> {
    if live.is_empty() || rules.is_empty() {
        return live;
    }
    let body = median(live.iter().map(|&g| all[g].size).collect());
    let mut rules = rules.to_vec();
    rules.sort_by(|a, b| a.w.total_cmp(&b.w));
    for r in &rules {
        if r.w > 0.6 * 612.0 {
            continue;
        }
        // Square root: a radical glyph whose top-right meets the rule's left
        // end.
        let rad: Vec<usize> = live
            .iter()
            .copied()
            .filter(|&g| {
                is_radical(&all[g].t)
                    && (all[g].x1 - r.x0).abs() < 1.5
                    && ((h - all[g].top) - r.y).abs() < 0.35 * all[g].size + 1.5
            })
            .collect();
        if !rad.is_empty() {
            let under: Vec<usize> = live
                .iter()
                .copied()
                .filter(|g| !rad.contains(g))
                .filter(|&g| {
                    r.x0 - 0.5 <= all[g].cx()
                        && all[g].cx() <= r.x1 + 0.5
                        && (h - all[g].top) <= r.y + 0.6
                        && r.y - (h - all[g].top) < 1.6 * all[g].size
                })
                .collect();
            if !under.is_empty() {
                let s = max_size(all, &under);
                let nb = median(
                    under
                        .iter()
                        .filter(|&&g| all[g].size >= 0.85 * s)
                        .map(|&g| all[g].base)
                        .collect(),
                );
                let under: Vec<usize> = under
                    .into_iter()
                    .filter(|&g| nb - 0.6 * s <= all[g].base && all[g].base <= nb + 0.8 * s)
                    .collect();
                let tex = format!(r"\sqrt{{{}}}", tex_of(all, &under));
                let parts: Vec<usize> = under.iter().chain(rad.iter()).copied().collect();
                let sy = stand_in(all, tex, &parts, all[rad[0]].x0, r.x1, nb, s);
                live = replace(all, live, &parts, sy);
                continue;
            }
        }
        let num = stack(all, &live, r, h, true);
        let den = stack(all, &live, r, h, false);
        let tol = (0.3 * body).max(3.0);
        if !num.is_empty() && !den.is_empty() {
            let (nx0, nx1) = width(all, &num);
            let (dx0, dx1) = width(all, &den);
            let wide = (nx1 - nx0).max(dx1 - dx0);
            let rc = (r.x0 + r.x1) / 2.0;
            let centred = ((nx0 + nx1) / 2.0 - rc).abs() < 0.15 * r.w + 1.5
                && ((dx0 + dx1) / 2.0 - rc).abs() < 0.15 * r.w + 1.5;
            if r.w - tol <= wide
                && wide <= r.w + 1.0
                && centred
                && !wordy(all, &num)
                && !wordy(all, &den)
            {
                let tex = format!(r"\frac{{{}}}{{{}}}", tex_of(all, &num), tex_of(all, &den));
                let parts: Vec<usize> = num.iter().chain(den.iter()).copied().collect();
                let part = max_size(all, &parts);
                // Note: display fractions set their parts at full size,
                // text-style ones at script size (so the fraction's own size
                // is ~1/0.7 of its parts').
                let size = if part >= 0.92 * body {
                    part
                } else {
                    body.min(part / 0.7)
                };
                let sy = stand_in(all, tex, &parts, r.x0, r.x1, r.y - 0.25 * size, size);
                live = replace(all, live, &parts, sy);
                continue;
            }
        }
        // Overline: math glyphs right under the rule, as wide as it.
        if !den.is_empty() {
            let (dx0, dx1) = width(all, &den);
            let close = den
                .iter()
                .any(|&g| r.y - (h - all[g].top) < 0.45 * all[g].size + 1.0);
            let top_g = den
                .iter()
                .copied()
                .reduce(|a, b| if all[b].top < all[a].top { b } else { a })
                .expect("non-empty");
            if close
                && den.iter().any(|&g| all[g].role.is_some())
                && ((dx1 - dx0) - r.w).abs() < (0.3 * body).max(2.5)
                && r.y - (h - all[top_g].top) < 0.5 * body
            {
                let tex = format!(r"\overline{{{}}}", tex_of(all, &den));
                let s = max_size(all, &den);
                let nb = median(
                    den.iter()
                        .filter(|&&g| all[g].size >= 0.85 * s)
                        .map(|&g| all[g].base)
                        .collect(),
                );
                let sy = stand_in(all, tex, &den, dx0, dx1, nb, s);
                live = replace(all, live, &den, sy);
            }
        }
    }
    live
}
