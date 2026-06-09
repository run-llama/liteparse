use crate::error::LiteParseError;
use crate::types::{ChartItem, ChartType, Page, ParsedPage, TextItem};
use pdfium::Document;
use std::collections::VecDeque;

const BAR_CHAR: char = '█';
const EMPTY_COLOR_KEY: u8 = u8::MAX;

#[derive(Debug, Clone)]
pub(crate) struct PageCharts {
    pub page_number: usize,
    pub charts: Vec<ChartItem>,
}

#[derive(Debug, Clone)]
struct Component {
    x1: usize,
    y1: usize,
    x2: usize,
    y2: usize,
    area: usize,
    color_key: u8,
}

impl Component {
    fn width(&self) -> usize {
        self.x2 - self.x1 + 1
    }

    fn height(&self) -> usize {
        self.y2 - self.y1 + 1
    }

    fn cx(&self) -> f32 {
        (self.x1 + self.x2) as f32 / 2.0
    }
}

/// Detect simple vector bar charts on already-extracted pages.
///
/// The detector is deliberately conservative: unsupported/low-confidence
/// visuals are ignored silently so normal parsed text is not degraded.
pub(crate) fn detect_charts_for_pages(
    document: &Document,
    pages: &[Page],
    dpi: f32,
) -> Result<Vec<PageCharts>, LiteParseError> {
    let mut out = Vec::new();
    for page in pages {
        let pdf_page = document.page((page.page_number - 1) as i32)?;
        let bitmap = pdf_page.render(dpi)?;
        if let Some(chart) = detect_bar_chart(page, &bitmap) {
            out.push(PageCharts {
                page_number: page.page_number,
                charts: vec![chart],
            });
        }
    }
    Ok(out)
}

pub(crate) fn apply_charts_to_parsed_pages(parsed_pages: &mut [ParsedPage], charts: &[PageCharts]) {
    for page_charts in charts {
        let Some(page) = parsed_pages
            .iter_mut()
            .find(|page| page.page_number == page_charts.page_number)
        else {
            continue;
        };

        for chart in &page_charts.charts {
            let Some(replaced) = replace_chart_outline(&page.text, chart) else {
                // Important fallback: do not append unsupported/uncertain chart text.
                continue;
            };
            page.text = replaced;
            page.charts.push(chart.clone());
        }
    }
}

fn detect_bar_chart(page: &Page, bitmap: &pdfium::Bitmap) -> Option<ChartItem> {
    let width = bitmap.width().max(0) as usize;
    let height = bitmap.height().max(0) as usize;
    if width == 0 || height == 0 || page.page_width <= 0.0 || page.page_height <= 0.0 {
        return None;
    }

    let scale_x = width as f32 / page.page_width;
    let scale_y = height as f32 / page.page_height;
    let color_keys = build_color_key_map(bitmap);
    let components = connected_components(&color_keys, width, height);
    let bars = filter_bar_components(components, width, height);
    if bars.len() < 2 {
        return None;
    }

    let clusters = cluster_bars(&bars);
    if clusters.len() < 2
        || clusters.iter().any(|cluster| cluster.is_empty())
        || !has_consistent_bar_groups(&clusters)
        || !has_common_baseline(&bars, height)
    {
        return None;
    }

    let (axis_a, axis_b, max_tick) = fit_y_axis(&page.text_items, &bars, scale_x, scale_y)?;
    if axis_a >= 0.0 || max_tick <= 0.0 {
        return None;
    }

    let (groups, series) = extract_labels(&page.text_items, &bars, &clusters, scale_x, scale_y);
    if groups.len() != clusters.len() || groups.iter().all(|label| label.starts_with("Group ")) {
        return None;
    }

    let series_count = clusters.iter().map(Vec::len).max().unwrap_or(0);
    if series_count == 0 {
        return None;
    }

    let mut values = Vec::with_capacity(clusters.len());
    for cluster in &clusters {
        let mut row = Vec::new();
        for bar in cluster {
            row.push((axis_a * bar.y1 as f32 + axis_b).max(0.0));
        }
        values.push(row);
    }

    let max_value = values
        .iter()
        .flatten()
        .copied()
        .fold(max_tick, f32::max)
        .max(1.0);
    let ascii = build_ascii_chart(&groups, &series, &values, max_value);

    let x1 = bars.iter().map(|bar| bar.x1).min().unwrap_or(0) as f32 / scale_x;
    let y1 = bars.iter().map(|bar| bar.y1).min().unwrap_or(0) as f32 / scale_y;
    let x2 = bars.iter().map(|bar| bar.x2).max().unwrap_or(0) as f32 / scale_x;
    let y2 = bars.iter().map(|bar| bar.y2).max().unwrap_or(0) as f32 / scale_y;

    Some(ChartItem {
        page_number: page.page_number,
        chart_type: ChartType::Bar,
        x: x1,
        y: y1,
        width: (x2 - x1).max(0.0),
        height: (y2 - y1).max(0.0),
        ascii,
        series,
        groups,
        values,
        max_value,
    })
}

fn build_color_key_map(bitmap: &pdfium::Bitmap) -> Vec<u8> {
    let width = bitmap.width().max(0) as usize;
    let height = bitmap.height().max(0) as usize;
    let stride = bitmap.stride().max(0) as usize;
    let buffer = bitmap.buffer();
    let mut keys = vec![EMPTY_COLOR_KEY; width * height];

    for y in 0..height {
        let row_start = y * stride;
        for x in 0..width {
            let offset = row_start + x * 4;
            if offset + 2 >= buffer.len() {
                continue;
            }
            // PDFium bitmap is BGRA.
            let b = buffer[offset];
            let g = buffer[offset + 1];
            let r = buffer[offset + 2];
            if is_saturated_color(r, g, b) {
                keys[y * width + x] = hue_bucket(r, g, b);
            }
        }
    }

    keys
}

fn is_saturated_color(r: u8, g: u8, b: u8) -> bool {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    max > 80 && max - min > 50 && !(r < 80 && g < 80 && b < 80)
}

fn hue_bucket(r: u8, g: u8, b: u8) -> u8 {
    let r = r as f32 / 255.0;
    let g = g as f32 / 255.0;
    let b = b as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    if delta <= f32::EPSILON {
        return 0;
    }

    let hue = if (max - r).abs() <= f32::EPSILON {
        ((g - b) / delta).rem_euclid(6.0)
    } else if (max - g).abs() <= f32::EPSILON {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    };
    ((hue * 6.0).round() as i32).rem_euclid(36) as u8
}

fn connected_components(keys: &[u8], width: usize, height: usize) -> Vec<Component> {
    let mut seen = vec![false; keys.len()];
    let mut out = Vec::new();

    for idx in 0..keys.len() {
        let key = keys[idx];
        if key == EMPTY_COLOR_KEY || seen[idx] {
            continue;
        }

        seen[idx] = true;
        let mut queue = VecDeque::from([idx]);
        let mut x1 = width;
        let mut y1 = height;
        let mut x2 = 0usize;
        let mut y2 = 0usize;
        let mut area = 0usize;

        while let Some(cur) = queue.pop_front() {
            area += 1;
            let x = cur % width;
            let y = cur / width;
            x1 = x1.min(x);
            y1 = y1.min(y);
            x2 = x2.max(x);
            y2 = y2.max(y);

            let mut push_neighbor = |next: usize| {
                if !seen[next] && keys[next] == key {
                    seen[next] = true;
                    queue.push_back(next);
                }
            };

            if x > 0 {
                push_neighbor(cur - 1);
            }
            if x + 1 < width {
                push_neighbor(cur + 1);
            }
            if y > 0 {
                push_neighbor(cur - width);
            }
            if y + 1 < height {
                push_neighbor(cur + width);
            }
        }

        out.push(Component {
            x1,
            y1,
            x2,
            y2,
            area,
            color_key: key,
        });
    }

    out
}

fn filter_bar_components(
    components: Vec<Component>,
    width: usize,
    height: usize,
) -> Vec<Component> {
    let min_area = ((width * height) as f32 * 0.00012).max(120.0) as usize;
    let min_height = (height as f32 * 0.015).max(12.0) as usize;
    let min_width = (width as f32 * 0.006).max(5.0) as usize;

    let mut bars: Vec<_> = components
        .into_iter()
        .filter(|c| {
            c.area >= min_area
                && c.height() >= min_height
                && c.width() >= min_width
                && c.height() > c.width()
        })
        .collect();
    bars.sort_by(|a, b| {
        a.cx()
            .total_cmp(&b.cx())
            .then(a.color_key.cmp(&b.color_key))
    });
    bars
}

fn cluster_bars(bars: &[Component]) -> Vec<Vec<Component>> {
    if bars.is_empty() {
        return Vec::new();
    }

    let mut widths: Vec<f32> = bars.iter().map(|bar| bar.width() as f32).collect();
    widths.sort_by(f32::total_cmp);
    let median_width = widths[widths.len() / 2];
    let threshold = (median_width * 1.9).max(28.0);

    let mut clusters: Vec<Vec<Component>> = Vec::new();
    for bar in bars.iter().cloned() {
        let same_cluster = clusters
            .last()
            .and_then(|cluster| cluster.last())
            .is_some_and(|prev| bar.cx() - prev.cx() <= threshold);
        if same_cluster {
            clusters.last_mut().unwrap().push(bar);
        } else {
            clusters.push(vec![bar]);
        }
    }

    clusters
}

fn has_consistent_bar_groups(clusters: &[Vec<Component>]) -> bool {
    if clusters.len() < 2
        || clusters
            .iter()
            .any(|cluster| cluster.is_empty() || cluster.len() > 8)
    {
        return false;
    }

    let counts: Vec<usize> = clusters.iter().map(Vec::len).collect();
    let min = counts.iter().copied().min().unwrap_or(0);
    let max = counts.iter().copied().max().unwrap_or(0);
    max.saturating_sub(min) <= 1
}

fn has_common_baseline(bars: &[Component], page_height_px: usize) -> bool {
    if bars.len() < 2 {
        return false;
    }

    let mut bottoms: Vec<usize> = bars.iter().map(|bar| bar.y2).collect();
    bottoms.sort_unstable();
    let median_bottom = bottoms[bottoms.len() / 2];
    let tolerance = ((page_height_px as f32) * 0.025).max(8.0) as usize;

    bottoms
        .iter()
        .filter(|bottom| bottom.abs_diff(median_bottom) <= tolerance)
        .count()
        * 2
        >= bottoms.len()
}

fn fit_y_axis(
    text_items: &[TextItem],
    bars: &[Component],
    scale_x: f32,
    scale_y: f32,
) -> Option<(f32, f32, f32)> {
    let chart_x1 = bars.iter().map(|bar| bar.x1).min()? as f32;
    let chart_y1 = bars.iter().map(|bar| bar.y1).min()? as f32;
    let chart_y2 = bars.iter().map(|bar| bar.y2).max()? as f32;

    let mut ticks = Vec::new();
    for item in text_items {
        let text = item.text.trim();
        let Ok(value) = text.parse::<f32>() else {
            continue;
        };
        if !value.is_finite() || value < 0.0 {
            continue;
        }
        let cx = (item.x + item.width / 2.0) * scale_x;
        let cy = (item.y + item.height / 2.0) * scale_y;
        if cx < chart_x1 - 8.0 && cy >= chart_y1 - 180.0 && cy <= chart_y2 + 80.0 {
            ticks.push((value, cy));
        }
    }

    if ticks.len() < 2 {
        return None;
    }

    let n = ticks.len() as f32;
    let sum_y: f32 = ticks.iter().map(|(_, y)| *y).sum();
    let sum_v: f32 = ticks.iter().map(|(v, _)| *v).sum();
    let sum_yy: f32 = ticks.iter().map(|(_, y)| y * y).sum();
    let sum_yv: f32 = ticks.iter().map(|(v, y)| v * y).sum();
    let denom = n * sum_yy - sum_y * sum_y;
    if denom.abs() <= f32::EPSILON {
        return None;
    }

    let a = (n * sum_yv - sum_y * sum_v) / denom;
    let b = (sum_v - a * sum_y) / n;
    let max_tick = ticks.iter().map(|(v, _)| *v).fold(0.0, f32::max);
    Some((a, b, max_tick))
}

fn extract_labels(
    text_items: &[TextItem],
    bars: &[Component],
    clusters: &[Vec<Component>],
    scale_x: f32,
    scale_y: f32,
) -> (Vec<String>, Vec<String>) {
    let chart_x1 = bars.iter().map(|bar| bar.x1).min().unwrap_or(0) as f32;
    let chart_x2 = bars.iter().map(|bar| bar.x2).max().unwrap_or(0) as f32;
    let chart_y1 = bars.iter().map(|bar| bar.y1).min().unwrap_or(0) as f32;
    let chart_y2 = bars.iter().map(|bar| bar.y2).max().unwrap_or(0) as f32;

    let mut group_candidates: Vec<(f32, String)> = Vec::new();
    let mut legend_candidates: Vec<(f32, String)> = Vec::new();

    for item in text_items {
        let label = item.text.trim();
        if label.is_empty() || label.parse::<f32>().is_ok() {
            continue;
        }
        let cx = (item.x + item.width / 2.0) * scale_x;
        let cy = (item.y + item.height / 2.0) * scale_y;

        if cx >= chart_x1 - 120.0
            && cx <= chart_x2 + 120.0
            && cy >= chart_y2
            && cy <= chart_y2 + 110.0
        {
            group_candidates.push((cx, label.to_string()));
        }
        if cx > chart_x2 + 35.0 && cy >= chart_y1 - 110.0 && cy <= chart_y2 + 110.0 {
            legend_candidates.push((cy, label.to_string()));
        }
    }

    group_candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
    group_candidates.dedup_by(|a, b| a.1 == b.1);

    let mut groups = Vec::with_capacity(clusters.len());
    for (idx, cluster) in clusters.iter().enumerate() {
        let center = cluster.iter().map(Component::cx).sum::<f32>() / cluster.len() as f32;
        let label = group_candidates
            .iter()
            .min_by(|a, b| (a.0 - center).abs().total_cmp(&(b.0 - center).abs()))
            .map(|(_, label)| label.clone())
            .unwrap_or_else(|| format!("Group {}", idx + 1));
        groups.push(label);
    }

    legend_candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
    legend_candidates.dedup_by(|a, b| a.1 == b.1);
    let series_count = clusters.iter().map(Vec::len).max().unwrap_or(0);
    let mut series: Vec<String> = legend_candidates
        .into_iter()
        .take(series_count)
        .map(|(_, label)| label)
        .collect();
    while series.len() < series_count {
        series.push(format!("Series {}", series.len() + 1));
    }

    (groups, series)
}

fn build_ascii_chart(
    groups: &[String],
    series: &[String],
    values: &[Vec<f32>],
    max_value: f32,
) -> String {
    let height = max_value.round().clamp(8.0, 20.0) as usize;
    let group_w = (series.len() + 4).max(8);
    let plot_w = group_w * groups.len();
    let mut canvas = vec![vec![' '; plot_w]; height];

    for (group_idx, row) in values.iter().enumerate() {
        let base_x = group_idx * group_w + 2;
        for (series_idx, value) in row.iter().enumerate() {
            let bar_h = ((*value / max_value) * height as f32)
                .round()
                .clamp(1.0, height as f32) as usize;
            let x = base_x + series_idx;
            if x >= plot_w {
                continue;
            }
            for y in height - bar_h..height {
                canvas[y][x] = BAR_CHAR;
            }
        }
    }

    let mut lines = Vec::new();
    lines.push(format!("       +{}+", "-".repeat(plot_w)));
    for (y, row) in canvas.iter().enumerate() {
        let value = max_value * (height - y) as f32 / height as f32;
        let tick = if y == 0 || y == height - 1 || y % 2 == 0 {
            format!("{value:>4.0}")
        } else {
            "    ".to_string()
        };
        lines.push(format!("{tick}   |{}|", row.iter().collect::<String>()));
    }
    lines.push(format!("   0   +{}+", "-".repeat(plot_w)));
    lines.push(format!(
        "       {}",
        groups
            .iter()
            .map(|label| format!("{label:^group_w$}"))
            .collect::<String>()
            .trim_end()
    ));
    if !series.is_empty() {
        lines.push(format!(
            "       Bars in each group, left-to-right: {}",
            series.join(" | ")
        ));
    }
    lines.push(String::new());
    lines.push("Values:".to_string());
    for (group, row) in groups.iter().zip(values) {
        let parts = series
            .iter()
            .zip(row)
            .map(|(series, value)| format!("{series}={value:.1}"))
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(format!("  {group}: {parts}"));
    }

    lines.join("\n")
}

fn replace_chart_outline(page_text: &str, chart: &ChartItem) -> Option<String> {
    let lines: Vec<&str> = page_text.lines().collect();
    if lines.is_empty() || chart.groups.is_empty() {
        return None;
    }

    let min_labels = chart.groups.len().min(3);
    let row_line_idx = lines.iter().position(|line| {
        chart
            .groups
            .iter()
            .take(min_labels)
            .filter(|label| line.contains(label.as_str()))
            .count()
            >= min_labels.min(2)
    })?;

    let max_tick = if chart.max_value.fract().abs() < 0.05 {
        format!("{:.0}", chart.max_value)
    } else {
        format!("{:.1}", chart.max_value)
    };

    let search_start = row_line_idx.saturating_sub(24);
    let mut start_idx = None;
    for idx in (search_start..=row_line_idx).rev() {
        let line = lines[idx];
        if contains_number_token(line, &max_tick) {
            start_idx = Some(idx);
        }
    }

    let start_idx = start_idx.or_else(|| {
        (search_start..=row_line_idx).find(|idx| {
            lines[*idx]
                .split_whitespace()
                .any(|part| part.parse::<f32>().is_ok())
        })
    })?;

    if row_line_idx <= start_idx {
        return None;
    }

    let mut out = Vec::new();
    out.extend_from_slice(&lines[..start_idx]);
    out.extend(chart.ascii.lines());
    out.extend_from_slice(&lines[row_line_idx + 1..]);
    Some(out.join("\n") + "\n")
}

fn contains_number_token(line: &str, token: &str) -> bool {
    line.split_whitespace().any(|part| part == token)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn component(x1: usize, y1: usize, x2: usize, y2: usize) -> Component {
        Component {
            x1,
            y1,
            x2,
            y2,
            area: (x2 - x1 + 1) * (y2 - y1 + 1),
            color_key: 0,
        }
    }

    #[test]
    fn saturated_color_detects_colored_bars_not_grey() {
        assert!(is_saturated_color(50, 120, 220));
        assert!(!is_saturated_color(200, 200, 200));
        assert!(!is_saturated_color(20, 20, 20));
    }

    #[test]
    fn ascii_chart_uses_block_bars() {
        let ascii = build_ascii_chart(
            &["A".into(), "B".into()],
            &["S1".into(), "S2".into()],
            &[vec![3.0, 6.0], vec![8.0, 2.0]],
            10.0,
        );
        assert!(ascii.contains(BAR_CHAR));
        assert!(ascii.contains("Bars in each group, left-to-right: S1 | S2"));
    }

    #[test]
    fn replace_chart_outline_replaces_axis_block() {
        let chart = ChartItem {
            page_number: 1,
            chart_type: ChartType::Bar,
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
            ascii: "bar".into(),
            series: vec!["Column 1".into()],
            groups: vec!["Row 1".into(), "Row 2".into()],
            values: vec![vec![1.0], vec![2.0]],
            max_value: 12.0,
        };
        let text = "before\n  12\n   6 Column 1\n   0\n  Row 1 Row 2\nafter";
        let replaced = replace_chart_outline(text, &chart).unwrap();
        assert!(replaced.contains("bar"));
        assert!(!replaced.contains("  Row 1 Row 2"));
        assert!(replaced.contains("before"));
        assert!(replaced.contains("after"));
    }

    #[test]
    fn replace_chart_outline_returns_none_without_group_labels() {
        let chart = ChartItem {
            page_number: 1,
            chart_type: ChartType::Bar,
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
            ascii: "bar".into(),
            series: vec!["Column 1".into()],
            groups: vec!["Row 1".into(), "Row 2".into()],
            values: vec![vec![1.0], vec![2.0]],
            max_value: 12.0,
        };
        assert!(replace_chart_outline("plain text\nwithout chart", &chart).is_none());
    }

    #[test]
    fn bar_group_confidence_accepts_consistent_grouped_bars() {
        let clusters = vec![
            vec![component(10, 10, 15, 100), component(18, 40, 23, 100)],
            vec![component(50, 20, 55, 100), component(58, 35, 63, 100)],
            vec![component(90, 30, 95, 100), component(98, 45, 103, 100)],
        ];
        assert!(has_consistent_bar_groups(&clusters));
        assert!(has_common_baseline(
            &clusters.into_iter().flatten().collect::<Vec<_>>(),
            200
        ));
    }

    #[test]
    fn bar_group_confidence_rejects_scattered_visuals() {
        let clusters = vec![
            vec![component(10, 10, 15, 80)],
            vec![component(50, 20, 55, 140), component(58, 35, 63, 140)],
            vec![
                component(90, 30, 95, 190),
                component(98, 45, 103, 190),
                component(106, 55, 111, 190),
                component(114, 65, 119, 190),
            ],
        ];
        assert!(!has_consistent_bar_groups(&clusters));

        let no_common_baseline = vec![
            component(10, 10, 15, 70),
            component(30, 10, 35, 100),
            component(50, 10, 55, 130),
            component(70, 10, 75, 170),
        ];
        assert!(!has_common_baseline(&no_common_baseline, 200));
    }

    #[test]
    fn fit_y_axis_uses_numeric_tick_positions() {
        let bars = vec![component(200, 100, 210, 200), component(240, 150, 250, 200)];
        let text_items = vec![
            TextItem {
                text: "10".into(),
                x: 100.0,
                y: 95.0,
                width: 10.0,
                height: 10.0,
                ..Default::default()
            },
            TextItem {
                text: "0".into(),
                x: 100.0,
                y: 195.0,
                width: 10.0,
                height: 10.0,
                ..Default::default()
            },
        ];
        let (a, b, max_tick) = fit_y_axis(&text_items, &bars, 1.0, 1.0).unwrap();
        assert!((max_tick - 10.0).abs() < 0.01);
        assert!((a * 100.0 + b - 10.0).abs() < 0.01);
        assert!((a * 200.0 + b).abs() < 0.01);
    }
}
