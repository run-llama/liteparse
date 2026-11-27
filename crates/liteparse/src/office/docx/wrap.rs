//! Converts DrawingML wrapping contours into horizontal exclusion strips.
//! Strips conservatively cover polygon edges; their height is at most 0.5 pt
//! for ordinary page-sized objects. Tight wrapping fills internal gaps, while
//! through wrapping retains the polygon's even-odd interior intervals.

#[derive(Debug, Clone)]
pub struct Contour {
    pub points: Vec<(f64, f64)>,
    pub through: bool,
}

type Band = (f64, f64, f64, f64);

impl Contour {
    /// Returns page-coordinate (top, bottom, left, right) strips, including
    /// wrap distances. Invalid or excessively complex contours use the caller's
    /// rectangular fallback. Degenerate extents exclude no area.
    pub fn bands(&self, x: f64, y: f64, w: f64, h: f64, dist: [f64; 4]) -> Option<Vec<Band>> {
        if w <= 0.001 || h <= 0.001 {
            return Some(Vec::new());
        }
        if self.points.len() < 3
            || self.points.len() > 4096
            || self
                .points
                .iter()
                .any(|(x, y)| !x.is_finite() || !y.is_finite() || x.abs() > 16.0 || y.abs() > 16.0)
        {
            return None;
        }
        let points: Vec<_> = self
            .points
            .iter()
            .map(|(px, py)| (x + px * w, y + py * h))
            .collect();
        let lo = points.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
        let hi = points.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
        if hi - lo <= 0.001 {
            return Some(Vec::new());
        }
        // Include every vertex so each strip's intersected edges are linear.
        // Bound work for oversized objects while retaining their full extent.
        let steps = ((hi - lo) / 0.5).ceil().min(4096.0) as usize;
        let mut cuts: Vec<_> = (0..=steps)
            .map(|i| lo + (hi - lo) * i as f64 / steps as f64)
            .collect();
        cuts.extend(points.iter().map(|p| p.1));
        cuts.sort_by(f64::total_cmp);
        cuts.dedup_by(|a, b| (*a - *b).abs() < 1e-8);
        let intersections = |at: f64| {
            let mut xs = Vec::new();
            for i in 0..points.len() {
                let (ax, ay) = points[i];
                let (bx, by) = points[(i + 1) % points.len()];
                if (ay <= at && by > at) || (by <= at && ay > at) {
                    xs.push(ax + (at - ay) * (bx - ax) / (by - ay));
                }
            }
            xs.sort_by(f64::total_cmp);
            xs
        };
        let [top, bottom, left, right] = dist;
        let mut out: Vec<Band> = Vec::new();
        for ys in cuts.windows(2) {
            let epsilon = (ys[1] - ys[0]) * 1e-6;
            let a = intersections(ys[0] + epsilon);
            let b = intersections(ys[1] - epsilon);
            if a.is_empty() && b.is_empty() {
                continue;
            }
            if a.len() != b.len() || a.len() % 2 != 0 {
                return None;
            }
            let intervals: Vec<_> = if self.through {
                a.as_chunks::<2>()
                    .0
                    .iter()
                    .zip(b.as_chunks::<2>().0)
                    .map(|(a, b)| (a[0].min(b[0]), a[1].max(b[1])))
                    .collect()
            } else {
                vec![(a[0].min(b[0]), a[a.len() - 1].max(b[b.len() - 1]))]
            };
            for (l, r) in intervals {
                if r - l <= 1e-8 {
                    continue;
                }
                let band = (ys[0] - top, ys[1] + bottom, l - left, r + right);
                // Rectangular contours collapse to a single band.
                if let Some(prev) = out.last_mut().filter(|p| {
                    (p.2 - band.2).abs() < 1e-8
                        && (p.3 - band.3).abs() < 1e-8
                        && band.0 <= p.1 + 1e-8
                }) {
                    prev.1 = prev.1.max(band.1);
                } else {
                    out.push(band);
                }
            }
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests;
