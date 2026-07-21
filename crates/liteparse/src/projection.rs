use crate::types::*;
use std::collections::{BTreeMap, HashMap, HashSet};

const FLOATING_SPACES: usize = 2;
const COLUMN_SPACES: usize = 4;

// Flowing text detection constants
const FLOWING_MAX_TOTAL_ANCHORS: usize = 4;
const FLOWING_MAX_LEFT_ANCHORS: usize = 3;
const FLOWING_MIN_LINES: usize = 3;
const FLOWING_WIDE_LINE_RATIO: f32 = 0.5;
const FLOWING_WIDE_LINE_THRESHOLD: f32 = 0.6;
const FLOWING_COLUMN_GAP_MULTIPLIER: f32 = 4.0;
const FLOWING_MIN_LINE_ITEMS: usize = 3;
const FLOWING_SPACE_HEIGHT_RATIO: f32 = 0.15;
const FLOWING_SPACE_MIN_THRESHOLD: f32 = 0.3;
// Space threshold as a fraction of the line's median character advance
// (horizontal em proxy), used to bound the height-based threshold from above.
// `height*0.15` over-estimates the inter-word gap for tall display/heading
// fonts (their ascender+leading make them ~2.7× their advance vs ~2× for body
// text), so it sits above the genuine word gaps and swallows the spaces
// ("WhichJournalistsdoPeople"). 0.30 is the body-text crossover — body height ≈
// 2×advance, so `advance*0.30 ≈ height*0.15` — so taking the smaller of the two
// leaves body text unchanged and only bites tall fonts.
const FLOWING_SPACE_ADV_RATIO: f32 = 0.30;
const FLOWING_MAX_INDENT: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SnapKind {
    Left,
    Right,
    Center,
}

struct LineRange {
    start: usize,
    end: usize,
}

fn compute_median_textbox_size(items: &[ProjectedTextItem]) -> (f32, f32) {
    if items.is_empty() {
        return (0.0, 0.0);
    }

    // Match TS behavior: median width is computed as average character width.
    let mut widths: Vec<f32> = items
        .iter()
        .filter_map(|item| {
            if item.item.width <= 0.0 {
                return None;
            }
            let char_len = item.item.text.chars().count();
            if char_len == 0 {
                return None;
            }
            Some(item.item.width / char_len as f32)
        })
        .collect();
    let mut heights: Vec<f32> = items
        .iter()
        .filter_map(|item| {
            if item.item.height > 0.0 {
                Some(item.item.height)
            } else {
                None
            }
        })
        .collect();

    if widths.is_empty() {
        widths.push(1.0);
    }
    if heights.is_empty() {
        heights.push(1.0);
    }

    widths.sort_by(|a, b| a.total_cmp(b));
    heights.sort_by(|a, b| a.total_cmp(b));

    let width_mid = widths.len() / 2;
    let height_mid = heights.len() / 2;

    let median_width = if widths.len().is_multiple_of(2) {
        (widths[width_mid - 1] + widths[width_mid]) / 2.0
    } else {
        widths[width_mid]
    };

    let median_height = if heights.len().is_multiple_of(2) {
        (heights[height_mid - 1] + heights[height_mid]) / 2.0
    } else {
        heights[height_mid]
    };

    (median_width, median_height)
}

fn canonical_rotation(rotation: f32) -> i32 {
    let r = rotation.rem_euclid(360.0);

    let candidates = [0.0f32, 90.0, 180.0, 270.0];
    let mut best = 0.0f32;
    let mut best_delta = f32::INFINITY;
    for c in candidates {
        // Circular angular distance, so rotations just under 360° are treated
        // as near 0° (e.g. 359° is 1° from upright, not 89° from 270°).
        let raw = (r - c).abs();
        let delta = raw.min(360.0 - raw);
        if delta < best_delta {
            best_delta = delta;
            best = c;
        }
    }

    if best_delta <= 2.0 {
        best as i32
    } else {
        r.round() as i32
    }
}

fn handle_rotation_reading_order(items: &mut [ProjectedTextItem], page_height: f32) {
    if !items
        .iter()
        .any(|b| canonical_rotation(b.item.rotation) != 0)
    {
        return;
    }

    // Group all items by rotation value.
    let mut groups_by_rotation: HashMap<i32, Vec<usize>> = HashMap::new();
    for (idx, bbox) in items.iter().enumerate() {
        let r = canonical_rotation(bbox.item.rotation);
        groups_by_rotation.entry(r).or_default().push(idx);
    }

    // For 90/270° groups, further split into spatially distinct clusters by y proximity.
    // This prevents e.g. top and bottom pin labels from being merged into one group.
    let mut bbox_groups: Vec<Vec<usize>> = Vec::new();
    for (rot, mut group) in groups_by_rotation {
        group.sort_by(|a, b| items[*a].item.y.total_cmp(&items[*b].item.y));
        if (rot == 90 || rot == 270) && group.len() > 1 {
            // Split when y gap between consecutive items exceeds a threshold
            let max_h = group
                .iter()
                .map(|idx| items[*idx].item.height)
                .fold(0.0f32, |a, b| a.max(b));
            let gap_threshold = max_h * 3.0;
            let mut cluster: Vec<usize> = vec![group[0]];
            for i in 1..group.len() {
                let prev_bottom = items[group[i - 1]].item.y + items[group[i - 1]].item.height;
                let cur_top = items[group[i]].item.y;
                if cur_top - prev_bottom > gap_threshold {
                    bbox_groups.push(std::mem::take(&mut cluster));
                }
                cluster.push(group[i]);
            }
            if !cluster.is_empty() {
                bbox_groups.push(cluster);
            }
        } else {
            bbox_groups.push(group);
        }
    }

    // Sort each subgroup by y.
    for group in &mut bbox_groups {
        group.sort_by(|a, b| items[*a].item.y.total_cmp(&items[*b].item.y));
    }

    bbox_groups.sort_by(|a, b| {
        let min_x_a = a
            .iter()
            .map(|idx| items[*idx].item.x)
            .fold(f32::INFINITY, |acc, v| acc.min(v));
        let min_x_b = b
            .iter()
            .map(|idx| items[*idx].item.x)
            .fold(f32::INFINITY, |acc, v| acc.min(v));
        min_x_a.total_cmp(&min_x_b)
    });

    for group_idx in 0..bbox_groups.len() {
        let group = bbox_groups[group_idx].clone();
        if group.is_empty() {
            continue;
        }

        let group_rotation = canonical_rotation(items[group[0]].item.rotation);
        if group_rotation != 90 && group_rotation != 270 {
            continue;
        }

        // Long-span guard: a rotated group whose screen-space extent along its
        // reading direction spans a large fraction of the page is almost
        // always sidebar/marginalia (page footers, signature stamps, vertical
        // disclaimers on legal/scanned docs) — never an inline label. Force the
        // separate-block branch so it doesn't get flattened onto body lines.
        //
        // For 90/270° text the reading direction is screen-vertical, so the
        // group's vertical screen span is the relevant axis.
        let group_min_y = group
            .iter()
            .map(|idx| items[*idx].item.y)
            .fold(f32::INFINITY, f32::min);
        let group_max_y = group
            .iter()
            .map(|idx| items[*idx].item.y + items[*idx].item.height)
            .fold(f32::NEG_INFINITY, f32::max);
        let group_vspan = (group_max_y - group_min_y).max(0.0);
        let long_span = page_height > 0.0 && group_vspan > page_height * 0.4;

        // Check if non-rotated/other-rotated items visually overlap or are near this group.
        // Use a proximity margin so rotated labels in diagrams (e.g. pin diagrams)
        // that are close to but don't strictly overlap non-rotated items are kept inline.
        let mut global_overlap = false;
        'outer: for (other_idx, other_bbox) in items.iter().enumerate() {
            let other_rot = canonical_rotation(other_bbox.item.rotation);
            if other_rot == group_rotation {
                continue;
            }

            for group_item_idx in &group {
                if *group_item_idx == other_idx {
                    continue;
                }
                let b = &items[*group_item_idx].item;
                let o = &other_bbox.item;
                // Proximity margin: use the max height of both items
                let margin = b.height.max(o.height);
                // Proper range overlap with margin on y-axis
                let x_overlap = b.x < o.x + o.width && b.x + b.width > o.x;
                let y_overlap = b.y < o.y + o.height + margin && b.y + b.height + margin > o.y;
                if x_overlap && y_overlap {
                    global_overlap = true;
                    break 'outer;
                }
            }
        }

        if global_overlap && !long_span {
            // For 90/270° groups kept inline, compute a common y from the
            // group's average vertical midpoint so all labels land on one row.
            // Keep original w/h to preserve x-spacing between labels.
            if group_rotation == 90 || group_rotation == 270 {
                let avg_cy: f32 = group
                    .iter()
                    .map(|idx| items[*idx].item.y + items[*idx].item.height / 2.0)
                    .sum::<f32>()
                    / group.len() as f32;
                let avg_w: f32 = group.iter().map(|idx| items[*idx].item.width).sum::<f32>()
                    / group.len() as f32;
                let common_y = avg_cy - avg_w / 2.0;

                for idx in &group {
                    if items[*idx].d != 0.0 {
                        items[*idx].item.y += items[*idx].d;
                        items[*idx].d = 0.0;
                    }
                    items[*idx].item.y = common_y;
                    items[*idx].item.height = avg_w;
                    items[*idx].item.rotation = 0.0;
                    items[*idx].rotated = true;
                }
            } else {
                for idx in &group {
                    if items[*idx].d != 0.0 {
                        items[*idx].item.y += items[*idx].d;
                        items[*idx].d = 0.0;
                    }
                    items[*idx].item.rotation = 0.0;
                    items[*idx].rotated = true;
                }
            }
        } else {
            let group_max_x = group
                .iter()
                .map(|idx| items[*idx].item.x + items[*idx].item.width)
                .fold(f32::NEG_INFINITY, |acc, v| acc.max(v));

            let mut delta_y = 0.0f32;
            if group_idx != 0 {
                let previous_group = &bbox_groups[group_idx - 1];
                let previous_group_max_y = previous_group
                    .iter()
                    .map(|idx| items[*idx].item.y + items[*idx].item.height)
                    .fold(f32::NEG_INFINITY, |acc, v| acc.max(v));
                delta_y = previous_group_max_y + page_height;
            }

            if group_rotation == 90 {
                for idx in &group {
                    let new_x = items[*idx].item.y.round();
                    let new_y = items[*idx].item.x + delta_y;
                    let new_w = items[*idx].item.height;
                    let new_h = items[*idx].item.width;
                    items[*idx].item.x = new_x;
                    items[*idx].item.y = new_y;
                    items[*idx].item.width = new_w;
                    items[*idx].item.height = new_h;
                    items[*idx].item.rotation = 0.0;
                    items[*idx].rotated = true;
                }
            }

            if group_rotation == 270 {
                let max_y = group
                    .iter()
                    .map(|idx| items[*idx].item.y + items[*idx].item.height)
                    .fold(f32::NEG_INFINITY, |acc, v| acc.max(v));
                for idx in &group {
                    let new_x = (max_y - items[*idx].item.y - items[*idx].item.height).round();
                    let new_y = items[*idx].item.x + delta_y;
                    let new_w = items[*idx].item.height;
                    let new_h = items[*idx].item.width;
                    items[*idx].item.x = new_x;
                    items[*idx].item.y = new_y;
                    items[*idx].item.width = new_w;
                    items[*idx].item.height = new_h;
                    items[*idx].item.rotation = 0.0;
                    items[*idx].rotated = true;
                }
            }

            let global_delta = delta_y + group_max_x + page_height;
            for other_group in &bbox_groups[(group_idx + 1)..] {
                for idx in other_group {
                    let rot = canonical_rotation(items[*idx].item.rotation);
                    if rot == 90 || rot == 270 {
                        items[*idx].d += global_delta;
                    } else {
                        items[*idx].item.y += global_delta;
                    }
                }
            }
        }
    }

    // Handle 180-degree rotation conservatively.
    // Unlike TS, we don't have extractor-provided rx/ry fields, so normalize to unrotated
    // and preserve local ordering by x.
    for group in &bbox_groups {
        if group.is_empty() {
            continue;
        }
        let rotation = canonical_rotation(items[group[0]].item.rotation);
        if rotation == 180 {
            let mut sorted = group.clone();
            sorted.sort_by(|a, b| items[*a].item.x.total_cmp(&items[*b].item.x));
            for idx in sorted {
                items[idx].item.rotation = 0.0;
                items[idx].rotated = true;
            }
        }
    }

    items.sort_by(|a, b| a.item.y.total_cmp(&b.item.y));
}

fn clean_projected_items(items: &mut Vec<ProjectedTextItem>, page_width: f32) {
    // Rust equivalent of cleanRawText margin cleanup.
    // Keep this conservative: only remove likely margin line numbers when they appear isolated.
    let midpoint = page_width * 0.5;
    let margin_left = midpoint - 5.0;
    let margin_right = midpoint + 20.0;

    let mut has_non_margin_by_line: HashMap<i32, bool> = HashMap::new();
    for item in items.iter() {
        let line_key = item.item.y.round() as i32;
        if !item.is_margin_line_number {
            has_non_margin_by_line.insert(line_key, true);
        }
    }

    items.retain(|item| {
        let line_key = item.item.y.round() as i32;
        let line_has_content = has_non_margin_by_line
            .get(&line_key)
            .copied()
            .unwrap_or(false);
        let center = item.item.x + item.item.width * 0.5;
        let text = item.item.text.trim();
        let looks_like_line_number = {
            let chars: Vec<char> = text.chars().collect();
            if chars.is_empty() || chars.len() > 3 {
                false
            } else {
                let mut digit_count = 0usize;
                let mut valid = true;
                for (idx, c) in chars.iter().enumerate() {
                    if c.is_ascii_digit() {
                        digit_count += 1;
                    } else if *c == 'O' && idx == chars.len() - 1 {
                        // OCR confusion 0->O
                    } else {
                        valid = false;
                        break;
                    }
                }
                valid && (1..=2).contains(&digit_count)
            }
        };

        let likely_margin = item.is_margin_line_number
            || (center > margin_left
                && center < margin_right
                && looks_like_line_number
                && item.item.width < 15.0);

        !likely_margin || line_has_content
    });
}

fn form_lines(
    items: &mut Vec<ProjectedTextItem>,
    median_width: f32,
    median_height: f32,
    page_width: f32,
) -> Vec<Vec<ProjectedTextItem>> {
    // Y-tolerance for sorting: items within this threshold are considered same line
    let y_sort_tolerance: f32 = (median_height * 0.5).max(5.0);

    let dbg_lines = std::env::var("LITEPARSE_DEBUG_LINES").is_ok();
    let dump_lines = |stage: &str, lines: &[Vec<ProjectedTextItem>]| {
        if !dbg_lines {
            return;
        }
        eprintln!(
            "[form_lines:{stage}] {} lines (mh={median_height:.1})",
            lines.len()
        );
        for (li, ln) in lines.iter().enumerate() {
            if ln.is_empty() {
                continue;
            }
            let min_y = ln.iter().map(|i| i.item.y).fold(f32::INFINITY, f32::min);
            let max_y = ln
                .iter()
                .map(|i| i.item.y + i.item.height)
                .fold(f32::NEG_INFINITY, f32::max);
            let txt: String = ln
                .iter()
                .map(|i| i.item.text.trim())
                .collect::<Vec<_>>()
                .join("|");
            eprintln!(
                "  [{li:2}] y={min_y:.1}..{max_y:.1} n={} {:.70}",
                ln.len(),
                txt
            );
        }
    };

    // For two-column documents, detect and mark margin line numbers
    if page_width > 0.0 {
        let midpoint = page_width / 2.0;
        let margin_left = midpoint - 5.0;
        let margin_right = midpoint + 20.0;

        fn is_margin_line_number_text(text: &str) -> bool {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return false;
            }
            let chars: Vec<char> = trimmed.chars().collect();
            if chars.len() > 3 {
                return false;
            }

            let mut digit_count = 0usize;
            for (idx, c) in chars.iter().enumerate() {
                if c.is_ascii_digit() {
                    digit_count += 1;
                } else if *c == 'O' && idx == chars.len() - 1 {
                    // OCR confusion: 0 -> O
                } else {
                    return false;
                }
            }
            (1..=2).contains(&digit_count)
        }

        for item in items.iter_mut() {
            let center = item.item.x + item.item.width / 2.0;

            if center > margin_left
                && center < margin_right
                && is_margin_line_number_text(&item.item.text)
                && item.item.width < 15.0
            {
                item.is_margin_line_number = true;
            }
        }
    }

    // Sort by y then x. Snap y to a grid so items on the same visual line
    // get identical keys — this keeps the comparator transitive (total order).
    let snap_y = |y: f32| -> i64 {
        if y_sort_tolerance > 0.0 {
            (y / y_sort_tolerance).round() as i64
        } else {
            (y * 1000.0).round() as i64
        }
    };
    items.sort_by(|a, b| {
        let ya = snap_y(a.item.y);
        let yb = snap_y(b.item.y);
        ya.cmp(&yb).then_with(|| a.item.x.total_cmp(&b.item.x))
    });

    fn can_merge(
        prev: &ProjectedTextItem,
        cur: &ProjectedTextItem,
        y_tolerance: f32,
        h_tolerance: f32,
    ) -> bool {
        if (cur.item.y - prev.item.y).abs() <= y_tolerance
            && (cur.item.height - prev.item.height).abs() <= h_tolerance
        {
            let delta_x = cur.item.x - (prev.item.x + prev.item.width);
            return (-0.5..0.0).contains(&delta_x) || (0.0..0.1).contains(&delta_x);
        }

        false
    }

    fn merge_bbox(prev: &ProjectedTextItem, cur: &ProjectedTextItem) -> (f32, f32, f32, f32) {
        let x1 = prev.item.x.min(cur.item.x);
        let y1 = prev.item.y.min(cur.item.y);
        let x2 = (prev.item.x + prev.item.width).max(cur.item.x + cur.item.width);
        let y2 = (prev.item.y + prev.item.height).max(cur.item.y + cur.item.height);
        (x1, y1, x2 - x1, y2 - y1)
    }

    fn merge_orig_bbox(prev: &mut ProjectedTextItem, cur: &ProjectedTextItem) {
        let x1 = prev.orig_x.min(cur.orig_x);
        let y1 = prev.orig_y.min(cur.orig_y);
        let x2 = (prev.orig_x + prev.orig_width).max(cur.orig_x + cur.orig_width);
        let y2 = (prev.orig_y + prev.orig_height).max(cur.orig_y + cur.orig_height);

        prev.orig_x = x1;
        prev.orig_y = y1;
        prev.orig_width = x2 - x1;
        prev.orig_height = y2 - y1;
    }

    // Merge continuous bbox items in a single linear pass.
    let merge_y_tolerance = 0.1;
    let merge_h_tolerance = 0.1;

    let mut merged_items: Vec<ProjectedTextItem> = Vec::with_capacity(items.len());
    for mut cur in items.drain(..) {
        let should_merge = merged_items
            .last()
            .map(|prev| can_merge(prev, &cur, merge_y_tolerance, merge_h_tolerance))
            .unwrap_or(false);

        if should_merge {
            if let Some(prev) = merged_items.last_mut() {
                let merged = merge_bbox(prev, &cur);
                prev.item.text.push_str(&cur.item.text);
                prev.item.words.append(&mut cur.item.words);
                prev.item.x = merged.0;
                prev.item.y = merged.1;
                prev.item.width = merged.2;
                prev.item.height = merged.3;
                merge_orig_bbox(prev, &cur);
            }
        } else {
            merged_items.push(cur);
        }
    }

    *items = merged_items;

    // try to find the bounding box that forms a line and group items by line
    let mut lines: Vec<Vec<ProjectedTextItem>> = Vec::new();
    let mut current_line: Vec<ProjectedTextItem> = Vec::new();
    let mut current_line_min_y = f32::INFINITY;
    let mut current_line_max_y = f32::NEG_INFINITY;
    for item in items.drain(..) {
        if !current_line.is_empty() {
            let mut line_collide = false;
            for line_item in current_line.iter() {
                let overlap_length = (line_item.item.x + line_item.item.width)
                    .min(item.item.x + item.item.width)
                    - line_item.item.x.max(item.item.x);

                if overlap_length > f32::max(5.0, median_width / 3.0) {
                    line_collide = true;
                    break;
                }
            }

            // Don't merge margin line numbers with regular content
            let cur_line_has_margin = current_line.iter().any(|i| i.is_margin_line_number);
            let cur_item_has_margin = item.is_margin_line_number;
            let margin_mismatch = cur_line_has_margin != cur_item_has_margin;

            // For rotated text, use y-tolerance based merging since heights may be inconsistent
            let y_tolerance_merge = if item.rotated {
                (median_height * 2.0).max(20.0)
            } else {
                0.0
            };
            let y_within_tolerance =
                item.rotated && (item.item.y - current_line_min_y).abs() < y_tolerance_merge;

            // Prevent "snowball" effect: when two columns have slightly offset
            // y-values, the line range keeps expanding as items from alternating
            // columns are added, merging multiple visual rows into one mega-line.
            // Cap the line height to a reasonable multiple of median text height.
            let proposed_min_y = current_line_min_y.min(item.item.y);
            let proposed_max_y = current_line_max_y.max(item.item.y + item.item.height);
            // too_tall threshold: prefer the smallest *trusted* item height
            // already in the current line as the line-height baseline. PDFium
            // reports inflated em-box heights for items with matrix-baked
            // sizing (fs=1.0); using the global `median_height` then permits
            // a real heading row to absorb the next visual row below it.
            // Falls back to median_height when no smaller anchor is available.
            let line_anchor_h = current_line
                .iter()
                .filter_map(|i| {
                    if matches!(i.item.font_size, Some(fs) if fs > 1.5)
                        || i.item.height < median_height
                    {
                        Some(i.item.height)
                    } else {
                        None
                    }
                })
                .fold(f32::INFINITY, f32::min);
            let too_tall_threshold = if line_anchor_h.is_finite() {
                (line_anchor_h * 1.8).max(median_height * 0.6)
            } else {
                median_height * 1.8
            };
            let too_tall = (proposed_max_y - proposed_min_y) > too_tall_threshold;

            if !line_collide
                && !margin_mismatch
                && !too_tall
                && (y_within_tolerance
                    || (item.item.y + item.item.height * 0.5 >= current_line_min_y
                        && item.item.y + item.item.height * 0.5 <= current_line_max_y)
                    || (item.item.y >= current_line_min_y && item.item.y <= current_line_max_y))
            {
                current_line_min_y = current_line_min_y.min(item.item.y);
                current_line_max_y = current_line_max_y.max(item.item.y + item.item.height);
                current_line.push(item);
            } else {
                lines.push(std::mem::take(&mut current_line));
                current_line_min_y = item.item.y;
                current_line_max_y = item.item.y + item.item.height;
                current_line.push(item);
            }
        } else {
            current_line_min_y = item.item.y;
            current_line_max_y = item.item.y + item.item.height;
            current_line.push(item);
        }
    }

    if !current_line.is_empty() {
        lines.push(current_line);
    }

    // sort each line by x
    for line in lines.iter_mut() {
        line.sort_by(|a, b| a.item.x.total_cmp(&b.item.x));
    }

    // sort lines by y
    lines.sort_by(|a, b| {
        let ay = a.first().map(|v| v.item.y).unwrap_or(f32::INFINITY);
        let by = b.first().map(|v| v.item.y).unwrap_or(f32::INFINITY);
        ay.total_cmp(&by)
    });

    dump_lines("after_group", &lines);

    // merge 'words'
    const MERGE_THRESHOLD: f32 = 1.0;

    fn looks_like_table_number(text: &str) -> bool {
        // Strip trailing statistical-significance markers ("0.77**", "0.31 *")
        // so decorated correlation/p-value cells still count as numbers. Without
        // this, the `both_are_numbers` guard below misfires and two adjacent
        // stat cells get merged into one, destroying the table's column tracks.
        let trimmed = text
            .trim()
            .trim_end_matches(|c: char| c == '*' || c == '†' || c == '‡' || c.is_whitespace());
        if trimmed.chars().count() < 2 {
            return false;
        }

        let mut chars = trimmed.chars().peekable();
        if matches!(chars.peek(), Some('$')) {
            chars.next();
        }
        if matches!(chars.peek(), Some('-')) {
            chars.next();
        }

        let mut has_digit = false;
        let mut has_decimal = false;
        for c in chars {
            if c.is_ascii_digit() {
                has_digit = true;
            } else if c == ',' {
                continue;
            } else if c == '.' {
                if has_decimal {
                    return false;
                }
                has_decimal = true;
            } else if c == '%' {
                return has_digit && trimmed.ends_with('%');
            } else {
                return false;
            }
        }

        has_digit
    }

    for line in lines.iter_mut() {
        let mut merged_line: Vec<ProjectedTextItem> = Vec::with_capacity(line.len());
        for mut item in line.drain(..) {
            if let Some(prev) = merged_line.last_mut() {
                let both_are_numbers = looks_like_table_number(&prev.item.text)
                    && looks_like_table_number(&item.item.text);

                let delta_x = item.item.x - prev.item.x - prev.item.width;
                // Don't merge items with noticeably different y positions (>1.5px).
                // Items on the same baseline typically differ by <0.5px.
                let y_diff = (item.item.y - prev.item.y).abs();
                let y_compatible = y_diff <= 1.5;

                // Decimal-fragment continuation: PDFium sometimes emits the
                // fractional part of a number ("15.6", "200.4") as a separate,
                // x-contiguous run beginning with the decimal point (".6").
                // `both_are_numbers` below treats them as two distinct table
                // values and keeps them apart, leaving "15 .6" in the rendered
                // cell and injecting a spurious column track between them. A
                // real pair of adjacent table numbers always has a visible
                // inter-cell gap, never a digit butted directly against a
                // leading-decimal run, so this is safe to join silently.
                let is_decimal_continuation = y_compatible
                    && (-2.0..=1.0).contains(&delta_x)
                    && prev
                        .item
                        .text
                        .chars()
                        .last()
                        .is_some_and(|c| c.is_ascii_digit())
                    && {
                        let mut cs = item.item.text.chars();
                        cs.next() == Some('.') && cs.next().is_some_and(|c| c.is_ascii_digit())
                    };
                if is_decimal_continuation {
                    prev.item.width = item.item.x + item.item.width - prev.item.x;
                    prev.item.text.push_str(&item.item.text);
                    prev.item.words.append(&mut item.item.words);
                    merge_orig_bbox(prev, &item);
                    continue;
                }

                // Word-boundary guard: when prev ends with an alphabetic
                // character and item starts with one, a positive delta_x is
                // very likely a missing-space gap (PDFium omitted the space
                // glyph between two words), not a deliberate silent join.
                // Silent-merging here fuses "of the" → "ofthe". Skip the
                // silent path so the items stay separate and the line
                // builder inserts a space based on `gap > space_threshold`.
                // Punctuation joins (`Standards`+`.`, `width`+`)`, etc.)
                // and overlapping/negative-gap merges still go through.
                let alpha_alpha_gap = delta_x > 0.0
                    && prev
                        .item
                        .text
                        .chars()
                        .last()
                        .is_some_and(|c| c.is_alphabetic())
                    && item
                        .item
                        .text
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_alphabetic());
                if y_compatible
                    && !both_are_numbers
                    && !alpha_alpha_gap
                    && delta_x <= MERGE_THRESHOLD
                {
                    prev.item.width = item.item.x + item.item.width - prev.item.x;
                    prev.item.text.push_str(&item.item.text);
                    prev.item.words.append(&mut item.item.words);
                    merge_orig_bbox(prev, &item);
                    continue;
                }

                let prev_len = prev.item.text.chars().count().max(1) as f32;
                let avg_char_width = prev.item.width / prev_len;
                if y_compatible && !both_are_numbers && delta_x < avg_char_width {
                    prev.item.width = item.item.x + item.item.width - prev.item.x;
                    if !prev.item.text.ends_with(' ') {
                        prev.item.text.push(' ');
                    }
                    prev.item.text.push_str(&item.item.text);
                    prev.item.words.append(&mut item.item.words);
                    merge_orig_bbox(prev, &item);
                    continue;
                }
            }

            merged_line.push(item);
        }

        *line = merged_line;
    }

    // Merge overlapping lines when there is no horizontal bbox overlap.
    let mut i = 1usize;
    while i < lines.len() {
        let (previous_min_y, previous_max_y) = {
            let previous = &lines[i - 1];
            let min_y = previous
                .iter()
                .map(|v| v.item.y)
                .fold(f32::INFINITY, |a, b| a.min(b));
            let max_y = previous
                .iter()
                .map(|v| v.item.y + v.item.height)
                .fold(f32::NEG_INFINITY, |a, b| a.max(b));
            (min_y, max_y)
        };

        let (current_min_y, current_max_y) = {
            let current = &lines[i];
            let min_y = current
                .iter()
                .map(|v| v.item.y)
                .fold(f32::INFINITY, |a, b| a.min(b));
            let max_y = current
                .iter()
                .map(|v| v.item.y + v.item.height)
                .fold(f32::NEG_INFINITY, |a, b| a.max(b));
            (min_y, max_y)
        };

        // Do the two lines overlap vertically?
        let lines_overlap = previous_max_y > current_min_y && previous_min_y < current_max_y;

        if lines_overlap {
            let bbox_overlap = {
                let previous = &lines[i - 1];
                let current = &lines[i];
                current.iter().any(|bbox| {
                    previous.iter().any(|prev_bbox| {
                        (bbox.item.x >= prev_bbox.item.x
                            && bbox.item.x <= prev_bbox.item.x + prev_bbox.item.width)
                            || (prev_bbox.item.x >= bbox.item.x
                                && prev_bbox.item.x <= bbox.item.x + bbox.item.width)
                    })
                })
            };

            if !bbox_overlap {
                if dbg_lines {
                    let pj: String = lines[i - 1]
                        .iter()
                        .map(|x| x.item.text.trim())
                        .collect::<Vec<_>>()
                        .join("|");
                    let cj: String = lines[i]
                        .iter()
                        .map(|x| x.item.text.trim())
                        .collect::<Vec<_>>()
                        .join("|");
                    eprintln!(
                        "[form_lines:overlap-merge] py={previous_min_y:.1}..{previous_max_y:.1} cy={current_min_y:.1}..{current_max_y:.1}\n    prev={pj:.60}\n    cur ={cj:.60}"
                    );
                }
                let mut current = lines.remove(i);
                lines[i - 1].append(&mut current);
                lines[i - 1].sort_by(|a, b| a.item.x.total_cmp(&b.item.x));
                continue;
            }
        }

        i += 1;
    }

    dump_lines("after_overlap_merge", &lines);

    // Insert blank lines for vertical gaps between lines.
    let mut i = 1;
    while i < lines.len() {
        let prev_metrics = representative_line_metrics(&lines[i - 1], median_height);
        let cur_metrics = representative_line_metrics(&lines[i], median_height);
        let y_delta = cur_metrics.0 - prev_metrics.1; // cur_top - prev_bottom
        let reference_height = median_height.max(prev_metrics.2.min(cur_metrics.2));

        if y_delta > reference_height {
            let num_blank = ((y_delta / reference_height).round() as usize).saturating_sub(1);
            let to_insert = num_blank.clamp(1, 10);
            for _ in 0..to_insert {
                lines.insert(i, Vec::new());
                i += 1;
            }
        }
        i += 1;
    }

    lines
}

/// Returns (top, bottom, height) for representative items in a line,
/// filtering out items much shorter than median height.
fn representative_line_metrics(
    line: &[ProjectedTextItem],
    global_median_height: f32,
) -> (f32, f32, f32) {
    if line.is_empty() {
        return (0.0, 0.0, 0.0);
    }

    let min_representative = global_median_height * 0.5;
    let has_representative = line.iter().any(|b| b.item.height >= min_representative);

    let top = line
        .iter()
        .filter(|b| !has_representative || b.item.height >= min_representative)
        .map(|b| b.item.y)
        .fold(f32::INFINITY, f32::min);
    let bottom = line
        .iter()
        .filter(|b| !has_representative || b.item.height >= min_representative)
        .map(|b| b.item.y + b.item.height)
        .fold(f32::NEG_INFINITY, f32::max);
    (top, bottom, bottom - top)
}

#[derive(Clone, Debug, Default)]
struct BoxMeta {
    left_anchor: Option<i32>,
    right_anchor: Option<i32>,
    center_anchor: Option<i32>,
    snap: Option<SnapKind>,
    should_space: usize,
    force_unsnapped: bool,
    rendered: bool,
    projected_x: usize,
}

fn anchor_key(x: f32) -> i32 {
    (x * 4.0).round() as i32
}

fn anchor_to_x(key: i32) -> f32 {
    key as f32 / 4.0
}

/// Visual (character) length of a string, not byte length.
/// Use this for column calculations instead of `.len()`.
fn char_len(s: &str) -> usize {
    s.chars().count()
}

fn trim_end_len(s: &str) -> usize {
    char_len(s.trim_end())
}

fn trim_end_in_place(s: &mut String) {
    let trimmed = s.trim_end().len(); // byte len for truncate
    s.truncate(trimmed);
}

fn line_space_end(raw_line: &str, should_space: usize) -> usize {
    let mut space_end = 0usize;
    if !raw_line.ends_with(' ') {
        space_end = should_space;
    }
    if should_space > 1 {
        let trailing_spaces = char_len(raw_line).saturating_sub(trim_end_len(raw_line));
        if trailing_spaces < should_space {
            space_end = should_space - trailing_spaces;
        }
    }
    space_end
}

fn can_render_bbox(meta_line: &[BoxMeta], idx: usize) -> bool {
    for m in meta_line.iter().take(idx) {
        if !m.rendered {
            return false;
        }
    }
    true
}

fn merge_nearby_anchor_groups(collection: &mut HashMap<i32, Vec<(usize, usize)>>) {
    const MERGE_TOLERANCE: i32 = 8; // 2 units in quarter-point anchor key space

    let sorted_keys: Vec<i32> = {
        let mut keys: Vec<i32> = collection.keys().copied().collect();
        keys.sort_unstable();
        keys
    };

    for (i, anchor) in sorted_keys.iter().enumerate() {
        if !collection.contains_key(anchor) {
            continue;
        }
        for next_anchor in sorted_keys.iter().skip(i + 1) {
            if !collection.contains_key(next_anchor) {
                continue;
            }
            if next_anchor - anchor > MERGE_TOLERANCE {
                break;
            }

            let current_len = collection.get(anchor).map(|v| v.len()).unwrap_or(0);
            let next_len = collection.get(next_anchor).map(|v| v.len()).unwrap_or(0);

            if next_len > current_len {
                if let Some(cur_items) = collection.remove(anchor)
                    && let Some(next_items) = collection.get_mut(next_anchor)
                {
                    next_items.extend(cur_items);
                }
                break;
            } else if let Some(next_items) = collection.remove(next_anchor)
                && let Some(cur_items) = collection.get_mut(anchor)
            {
                cur_items.extend(next_items);
            }
        }
    }
}

fn update_forward_anchor_right_bound(
    snap_map: &[i32],
    forward_anchor: &mut BTreeMap<i32, usize>,
    right_bound: i32,
    anchor_target: usize,
) {
    const POSITION_TOLERANCE: i32 = 8; // 2 units in quarter-point anchor key space

    for (idx, anchor) in snap_map.iter().enumerate().rev() {
        if *anchor < right_bound {
            return;
        }

        let entry = forward_anchor.entry(*anchor).or_insert(0);
        if anchor_target > *entry {
            *entry = anchor_target;
        }

        let mut j = idx;
        while j > 0 {
            let nearby_anchor = snap_map[j - 1];
            if *anchor - nearby_anchor > POSITION_TOLERANCE {
                break;
            }
            let nearby_entry = forward_anchor.entry(nearby_anchor).or_insert(0);
            if anchor_target > *nearby_entry {
                *nearby_entry = anchor_target;
            }
            j -= 1;
        }
    }
}

fn compress_wide_spaces(line: &str, min_run: usize, replace_with: usize) -> String {
    let mut out = String::with_capacity(line.len());
    let bytes = line.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b' ' {
            let start = i;
            while i < bytes.len() && bytes[i] == b' ' {
                i += 1;
            }
            let run_len = i - start;
            if run_len >= min_run {
                out.push_str(&" ".repeat(replace_with));
            } else {
                out.push_str(&" ".repeat(run_len));
            }
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

fn fix_sparse_blocks(raw_lines: &mut [String], start: usize, end: usize) {
    let mut total = 0usize;
    let mut whitespace = 0usize;

    for line in raw_lines.iter_mut().take(end).skip(start) {
        trim_end_in_place(line);
        if line.is_empty() {
            continue;
        }
        total += line.len();
        whitespace += line.chars().filter(|c| c.is_whitespace()).count();
    }

    if total >= 500 && (whitespace as f32 / total as f32) > 0.8 {
        for line in raw_lines.iter_mut().take(end).skip(start) {
            if line.is_empty() {
                continue;
            }
            *line = compress_wide_spaces(line, COLUMN_SPACES, FLOATING_SPACES);
        }
    }
}

// ---------------------------------------------------------------------------
// Block segmentation & flowing text detection
// ---------------------------------------------------------------------------

/// Segments page lines into blocks separated by double blank lines.
fn segment_blocks(lines: &[Vec<ProjectedTextItem>]) -> Vec<LineRange> {
    let mut blocks = Vec::new();
    let mut empty_count = 0usize;
    let mut start: Option<usize> = None;

    for (line_idx, line) in lines.iter().enumerate() {
        if line.is_empty() {
            empty_count += 1;
            if empty_count > 1 {
                if let Some(s) = start {
                    // Include the trailing double-blank at the end of the block
                    blocks.push(LineRange {
                        start: s,
                        end: line_idx + 1,
                    });
                }
                start = None;
            }
        } else {
            empty_count = 0;
            if start.is_none() {
                start = Some(line_idx);
            }
        }
    }

    if let Some(s) = start {
        blocks.push(LineRange {
            start: s,
            end: lines.len(),
        });
    }

    // If no blocks found, treat entire page as one block
    if blocks.is_empty() && !lines.is_empty() {
        blocks.push(LineRange {
            start: 0,
            end: lines.len(),
        });
    }

    blocks
}

/// Extract anchor maps for a single block of lines.
/// Returns (anchor_left, anchor_right, anchor_center) with absolute line indices.
fn extract_block_anchors(
    lines: &[Vec<ProjectedTextItem>],
    block: &LineRange,
) -> (AnchorMap, AnchorMap, AnchorMap) {
    let mut anchor_left: AnchorMap = HashMap::new();
    let mut anchor_right: AnchorMap = HashMap::new();
    let mut anchor_center: AnchorMap = HashMap::new();

    for (line_idx, line) in lines.iter().enumerate().take(block.end).skip(block.start) {
        for (box_idx, bbox) in line.iter().enumerate() {
            // Skip rotated items from anchor detection — their coordinates are
            // transformed and would create spurious column anchors that stretch
            // the layout. They'll render as floating items at their natural positions.
            if bbox.rotated {
                continue;
            }
            let left_key = anchor_key(bbox.item.x);
            let right_key = anchor_key(bbox.item.x + bbox.item.width);
            let center_key = anchor_key(bbox.item.x + bbox.item.width * 0.5);
            anchor_left
                .entry(left_key)
                .or_default()
                .push((line_idx, box_idx));
            anchor_right
                .entry(right_key)
                .or_default()
                .push((line_idx, box_idx));
            anchor_center
                .entry(center_key)
                .or_default()
                .push((line_idx, box_idx));
        }
    }

    (anchor_left, anchor_right, anchor_center)
}

/// Remove vertically isolated items from anchor groups.
/// Items must have a neighbor within `page_height * delta` to survive.
fn delta_min_filter(
    collection: &mut HashMap<i32, Vec<(usize, usize)>>,
    lines: &[Vec<ProjectedTextItem>],
    page_height: f32,
    delta: f32,
) {
    let max_delta = page_height * delta;

    for members in collection.values_mut() {
        // Sort members by y coordinate
        members.sort_by(|a, b| {
            let ya = lines[a.0][a.1].item.y;
            let yb = lines[b.0][b.1].item.y;
            ya.total_cmp(&yb)
        });

        let mut keep = vec![false; members.len()];
        for i in 0..members.len() {
            let y_cur = lines[members[i].0][members[i].1].item.y;
            if i > 0 {
                let y_prev = lines[members[i - 1].0][members[i - 1].1].item.y;
                if y_cur - y_prev < max_delta {
                    keep[i] = true;
                    keep[i - 1] = true;
                }
            }
            if i + 1 < members.len() {
                let y_next = lines[members[i + 1].0][members[i + 1].1].item.y;
                if y_next - y_cur < max_delta {
                    keep[i] = true;
                }
            }
        }

        let mut idx = 0;
        members.retain(|_| {
            let k = keep[idx];
            idx += 1;
            k
        });
    }

    collection.retain(|_, v| !v.is_empty());
}

/// Remove anchors where text from other items visually crosses the anchor x-position
/// between every consecutive pair of anchor members.
fn intercept_filter(
    collection: &mut HashMap<i32, Vec<(usize, usize)>>,
    lines: &[Vec<ProjectedTextItem>],
) {
    let anchors_to_remove: Vec<i32> = collection
        .iter()
        .filter_map(|(anchor_key_val, members)| {
            if members.len() < 2 {
                return None;
            }

            let anchor_x = anchor_to_x(*anchor_key_val);
            let mut any_pair_clear = false;

            for i in 1..members.len() {
                let y1 = lines[members[i - 1].0][members[i - 1].1].item.y;
                let y2 = lines[members[i].0][members[i].1].item.y;
                let (y_min, y_max) = if y1 < y2 { (y1, y2) } else { (y2, y1) };

                let mut intercepted = false;
                for line in lines.iter() {
                    if line.is_empty() {
                        continue;
                    }
                    let line_y = line[0].item.y;
                    if line_y > y_min && line_y < y_max {
                        for item in line {
                            if item.item.x < anchor_x && item.item.x + item.item.width > anchor_x {
                                intercepted = true;
                                break;
                            }
                        }
                        if intercepted {
                            break;
                        }
                    }
                }

                if !intercepted {
                    any_pair_clear = true;
                    break;
                }
            }

            if !any_pair_clear {
                Some(*anchor_key_val)
            } else {
                None
            }
        })
        .collect();

    for key in anchors_to_remove {
        collection.remove(&key);
    }
}

/// Try to align floating bboxes (not in any surviving anchor) to nearby anchors
/// on adjacent lines within the given margin.
fn try_align_floating(
    target: &mut HashMap<i32, Vec<(usize, usize)>>,
    lines: &[Vec<ProjectedTextItem>],
    block: &LineRange,
    anchored: &HashSet<(usize, usize)>,
    margin: f32,
    ref_x_fn: fn(&TextItem) -> f32,
    anchor_key_fn: fn(&TextItem) -> i32,
) {
    let mut additions: Vec<(i32, (usize, usize))> = Vec::new();

    for line_idx in block.start..block.end {
        for box_idx in 0..lines[line_idx].len() {
            if anchored.contains(&(line_idx, box_idx)) {
                continue;
            }
            // Skip rotated items — they are excluded from anchor extraction
            // and should remain floating to avoid spurious snap assignments.
            if lines[line_idx][box_idx].rotated {
                continue;
            }

            let ref_x = ref_x_fn(&lines[line_idx][box_idx].item);

            // Check adjacent lines for candidate anchors
            let mut candidate_anchor: Option<i32> = None;
            let mut prev_diff = margin + 1.0;

            let adj_lines: [Option<usize>; 2] = [
                if line_idx > block.start {
                    Some(line_idx - 1)
                } else {
                    None
                },
                if line_idx + 1 < block.end {
                    Some(line_idx + 1)
                } else {
                    None
                },
            ];

            for adj_opt in &adj_lines {
                let Some(adj_line_idx) = adj_opt else {
                    continue;
                };
                for adj_box in &lines[*adj_line_idx] {
                    let cand_key = anchor_key_fn(&adj_box.item);
                    if !target.contains_key(&cand_key) {
                        continue;
                    }
                    let x_diff = (anchor_to_x(cand_key) - ref_x).abs();
                    if x_diff <= margin && x_diff < prev_diff {
                        candidate_anchor = Some(cand_key);
                        prev_diff = x_diff;
                    }
                }
            }

            if let Some(key) = candidate_anchor {
                additions.push((key, (line_idx, box_idx)));
            }
        }
    }

    // Apply additions after iteration to avoid borrow conflicts
    for (key, item) in additions {
        if let Some(members) = target.get_mut(&key)
            && !members.contains(&item)
        {
            members.push(item);
        }
    }
}

/// Maximum horizontal gap between consecutive items on a line.
fn line_max_gap(line: &[ProjectedTextItem]) -> f32 {
    let mut max_gap: f32 = 0.0;
    for i in 1..line.len() {
        let gap = line[i].item.x - (line[i - 1].item.x + line[i - 1].item.width);
        if gap > max_gap {
            max_gap = gap;
        }
    }
    max_gap
}

/// Check if a line has a column-like gap: one gap that is much larger than
/// the typical inter-word gaps on the same line. This catches two-column
/// layouts where the absolute column gap is below the threshold but is
/// clearly an outlier relative to other gaps on the line.
fn line_has_column_gap(line: &[ProjectedTextItem], median_width: f32, page_width: f32) -> bool {
    if line.len() < 2 {
        return false;
    }
    let midpoint = page_width * 0.5;
    for i in 1..line.len() {
        let prev_end = line[i - 1].item.x + line[i - 1].item.width;
        let cur_start = line[i].item.x;
        let gap = cur_start - prev_end;
        // A gap is a column separator if it's large enough (>2x median char width)
        // AND items on either side span across the page midpoint.
        if gap > median_width * 2.0 && prev_end < midpoint && cur_start > midpoint {
            return true;
        }
    }
    false
}

/// Check if a block of lines is flowing paragraph text (vs structured/tabular).
fn is_flowing_text_block(
    lines: &[Vec<ProjectedTextItem>],
    block: &LineRange,
    anchor_left: &HashMap<i32, Vec<(usize, usize)>>,
    anchor_right: &HashMap<i32, Vec<(usize, usize)>>,
    anchor_center: &HashMap<i32, Vec<(usize, usize)>>,
    page_width: f32,
    median_width: f32,
) -> bool {
    let total_anchors = anchor_left.len() + anchor_right.len() + anchor_center.len();
    if total_anchors > FLOWING_MAX_TOTAL_ANCHORS {
        return false;
    }
    if anchor_left.len() > FLOWING_MAX_LEFT_ANCHORS {
        return false;
    }

    let mut non_empty_lines = 0usize;
    let mut wide_lines = 0usize;
    let mut column_gap_lines = 0usize;

    for line in lines.iter().take(block.end).skip(block.start) {
        if line.is_empty() {
            continue;
        }
        non_empty_lines += 1;

        let line_start = line[0].item.x;
        let line_end = line.last().map(|b| b.item.x + b.item.width).unwrap_or(0.0);
        if line_end - line_start > page_width * FLOWING_WIDE_LINE_RATIO {
            wide_lines += 1;
        }
        if line_has_column_gap(line, median_width, page_width) {
            column_gap_lines += 1;
        }
    }

    if non_empty_lines < FLOWING_MIN_LINES {
        return false;
    }

    // If multiple lines have column gaps, this is a multi-column block,
    // not flowing text.
    if column_gap_lines >= 2 {
        return false;
    }

    (wide_lines as f32 / non_empty_lines as f32) > FLOWING_WIDE_LINE_THRESHOLD
}

/// Render a single line as flowing text with indentation.
fn render_line_as_flowing_text(
    line: &mut [ProjectedTextItem],
    min_x: f32,
    median_width: f32,
    meta_line: &mut [BoxMeta],
) -> String {
    if line.is_empty() {
        return String::new();
    }

    let indent = ((line[0].item.x - min_x) / median_width)
        .round()
        .max(0.0)
        .min(FLOWING_MAX_INDENT as f32) as usize;

    let mut result = " ".repeat(indent);

    // Median per-glyph advance across the line, used to bound the space
    // threshold so tall display fonts don't get a threshold above their real
    // word gaps. `+inf` (no bound) when the line has no measurable advances.
    let adv_threshold = {
        let mut advs: Vec<f32> = line
            .iter()
            .filter_map(|it| {
                let n = it.item.text.chars().filter(|c| !c.is_whitespace()).count();
                (n > 0 && it.item.width > 0.0).then(|| it.item.width / n as f32)
            })
            .collect();
        if advs.is_empty() {
            f32::INFINITY
        } else {
            advs.sort_by(f32::total_cmp);
            advs[advs.len() / 2] * FLOWING_SPACE_ADV_RATIO
        }
    };

    for i in 0..line.len() {
        if i > 0 {
            let prev = &line[i - 1].item;
            let cur = &line[i].item;
            let gap = cur.x - (prev.x + prev.width);
            let space_threshold = (cur.height * FLOWING_SPACE_HEIGHT_RATIO)
                .min(adv_threshold)
                .max(FLOWING_SPACE_MIN_THRESHOLD);
            if gap > space_threshold && !result.ends_with(' ') {
                result.push(' ');
            }
        }
        result.push_str(&line[i].item.text);
        meta_line[i].rendered = true;
        line[i].rendered = true;
    }

    result
}

/// Render an entire block as flowing text (all lines).
fn render_flowing_block(
    lines: &mut [Vec<ProjectedTextItem>],
    block: &LineRange,
    raw_lines: &mut [String],
    meta: &mut [Vec<BoxMeta>],
    median_width: f32,
) {
    let mut min_x = f32::INFINITY;
    for line in lines.iter().take(block.end).skip(block.start) {
        if !line.is_empty() {
            min_x = min_x.min(line[0].item.x);
        }
    }
    if min_x == f32::INFINITY {
        min_x = 0.0;
    }

    for line_idx in block.start..block.end {
        if lines[line_idx].is_empty() {
            continue;
        }
        raw_lines[line_idx] = render_line_as_flowing_text(
            &mut lines[line_idx],
            min_x,
            median_width,
            &mut meta[line_idx],
        );
    }
}

/// Per-line flowing detection within structured blocks.
/// Identifies individual lines that should be rendered as flowing text.
fn detect_and_render_flowing_lines(
    lines: &mut [Vec<ProjectedTextItem>],
    block: &LineRange,
    raw_lines: &mut [String],
    meta: &mut [Vec<BoxMeta>],
    median_width: f32,
    page_width: f32,
) {
    let column_gap_threshold = median_width * FLOWING_COLUMN_GAP_MULTIPLIER;

    // Find block's left margin
    let mut block_min_x = f32::INFINITY;
    for line in lines.iter().take(block.end).skip(block.start) {
        if !line.is_empty() {
            block_min_x = block_min_x.min(line[0].item.x);
        }
    }
    if block_min_x == f32::INFINITY {
        block_min_x = 0.0;
    }

    let mut flowing_lines: HashSet<usize> = HashSet::new();

    // First pass: detect clearly flowing lines (wide span, no column gaps, enough items)
    for line_idx in block.start..block.end {
        let line = &lines[line_idx];
        if line.len() < FLOWING_MIN_LINE_ITEMS {
            continue;
        }

        let line_start = line[0].item.x;
        let line_end = line.last().map(|b| b.item.x + b.item.width).unwrap_or(0.0);
        let line_span = line_end - line_start;

        // Skip lines where items belong to multiple different snap groups
        // (e.g., left-column and right-column items on the same line bridged
        // by a margin line number)
        let has_mixed_snaps = {
            let mut first_snap: Option<SnapKind> = None;
            let mut mixed = false;
            for m in meta[line_idx].iter() {
                if let Some(s) = m.snap {
                    match first_snap {
                        None => first_snap = Some(s),
                        Some(fs) if fs != s => {
                            mixed = true;
                            break;
                        }
                        _ => {}
                    }
                }
            }
            mixed
        };

        if !has_mixed_snaps
            && line_span > page_width * FLOWING_WIDE_LINE_RATIO
            && line_max_gap(line) < column_gap_threshold
            && !line_has_column_gap(line, median_width, page_width)
        {
            flowing_lines.insert(line_idx);
        }
    }

    // Forward sweep: propagate flowing status downward
    for line_idx in block.start..block.end {
        let line = &lines[line_idx];
        if flowing_lines.contains(&line_idx) || line.is_empty() {
            continue;
        }
        let has_mixed_snaps = {
            let mut first_snap: Option<SnapKind> = None;
            let mut mixed = false;
            for m in meta[line_idx].iter() {
                if let Some(s) = m.snap {
                    match first_snap {
                        None => first_snap = Some(s),
                        Some(fs) if fs != s => {
                            mixed = true;
                            break;
                        }
                        _ => {}
                    }
                }
            }
            mixed
        };
        if !has_mixed_snaps
            && line_idx > block.start
            && flowing_lines.contains(&(line_idx - 1))
            && line_max_gap(line) < column_gap_threshold
            && !line_has_column_gap(line, median_width, page_width)
        {
            flowing_lines.insert(line_idx);
        }
    }

    // Backward sweep: propagate flowing status upward
    for line_idx in (block.start..block.end).rev() {
        if flowing_lines.contains(&line_idx) || lines[line_idx].is_empty() {
            continue;
        }
        let has_mixed_snaps = {
            let mut first_snap: Option<SnapKind> = None;
            let mut mixed = false;
            for m in meta[line_idx].iter() {
                if let Some(s) = m.snap {
                    match first_snap {
                        None => first_snap = Some(s),
                        Some(fs) if fs != s => {
                            mixed = true;
                            break;
                        }
                        _ => {}
                    }
                }
            }
            mixed
        };
        if !has_mixed_snaps
            && line_idx + 1 < block.end
            && flowing_lines.contains(&(line_idx + 1))
            && line_max_gap(&lines[line_idx]) < column_gap_threshold
            && !line_has_column_gap(&lines[line_idx], median_width, page_width)
        {
            flowing_lines.insert(line_idx);
        }
    }

    // Render flowing lines
    for &line_idx in &flowing_lines {
        raw_lines[line_idx] = render_line_as_flowing_text(
            &mut lines[line_idx],
            block_min_x,
            median_width,
            &mut meta[line_idx],
        );
    }
}

// ---------------------------------------------------------------------------
// Main grid projection
// ---------------------------------------------------------------------------

fn project_to_grid(
    page: &Page,
    mut projection_boxes: Vec<ProjectedTextItem>,
) -> (Vec<ProjectedTextItem>, String) {
    if projection_boxes.is_empty() {
        return (Vec::new(), String::new());
    }

    // Filter out items that are purely dots
    let mut dot_count = 0usize;
    projection_boxes.iter().for_each(|item| {
        if item
            .item
            .text
            .chars()
            .all(|c| c == '.' || c == '·' || c == '•')
        {
            dot_count += 1;
        }
    });

    if dot_count > 100 && (dot_count as f64) > (projection_boxes.len() as f64) * 0.05 {
        projection_boxes.retain(|item| {
            !item
                .item
                .text
                .chars()
                .all(|c| c == '.' || c == '·' || c == '•')
        });
    }

    // Round dimensions
    for item in projection_boxes.iter_mut() {
        item.item.width = item.item.width.round();
        item.item.height = item.item.height.round();
    }

    // Compute median distances
    let (median_width, median_height) = compute_median_textbox_size(&projection_boxes);

    // Handle reading order rotations
    handle_rotation_reading_order(&mut projection_boxes, page.page_height);

    // Form lines of boxes
    let mut lines = form_lines(
        &mut projection_boxes,
        median_width,
        median_height,
        page.page_width,
    );
    if lines.is_empty() {
        return (Vec::new(), String::new());
    }

    // Segment into blocks
    let blocks = segment_blocks(&lines);

    let debug = std::env::var("LITEPARSE_DEBUG").is_ok();
    if debug {
        eprintln!("[debug] median_width={median_width:.2}, median_height={median_height:.2}");
        eprintln!("[debug] {} blocks, {} lines", blocks.len(), lines.len());
        for (i, b) in blocks.iter().enumerate() {
            eprintln!("[debug] block {i}: lines {}-{}", b.start, b.end);
        }
    }

    let mut meta: Vec<Vec<BoxMeta>> = lines
        .iter()
        .map(|line| vec![BoxMeta::default(); line.len()])
        .collect();

    let mut raw_lines = vec![String::new(); lines.len()];

    // Page-scoped forward anchors (carry alignment across blocks)
    let mut forward_left: BTreeMap<i32, usize> = BTreeMap::new();
    let mut forward_right: BTreeMap<i32, usize> = BTreeMap::new();
    let mut forward_center: BTreeMap<i32, usize> = BTreeMap::new();
    let mut forward_floating: BTreeMap<i32, usize> = BTreeMap::new();

    for block in &blocks {
        // --- Anchor extraction (per block) ---
        let (mut anchor_left, mut anchor_right, mut anchor_center) =
            extract_block_anchors(&lines, block);

        merge_nearby_anchor_groups(&mut anchor_left);
        merge_nearby_anchor_groups(&mut anchor_right);
        merge_nearby_anchor_groups(&mut anchor_center);

        // Isolation filtering: remove vertically isolated anchor members
        delta_min_filter(&mut anchor_left, &lines, page.page_height, 0.25);
        delta_min_filter(&mut anchor_right, &lines, page.page_height, 0.17);
        delta_min_filter(&mut anchor_center, &lines, page.page_height, 0.05);

        // Intercept filtering: remove anchors crossed by other text
        intercept_filter(&mut anchor_left, &lines);
        intercept_filter(&mut anchor_right, &lines);
        intercept_filter(&mut anchor_center, &lines);

        // Try to align floating bboxes to nearby existing anchors
        // Align floating items to nearby anchors. Use type-specific skip sets
        // so items with e.g. only a left anchor can still be aligned to a right anchor.
        let left_anchored: HashSet<(usize, usize)> = anchor_left
            .values()
            .flat_map(|v| v.iter().copied())
            .collect();
        try_align_floating(
            &mut anchor_left,
            &lines,
            block,
            &left_anchored,
            4.0,
            |item| item.x,
            |item| anchor_key(item.x),
        );
        let right_anchored: HashSet<(usize, usize)> = anchor_right
            .values()
            .flat_map(|v| v.iter().copied())
            .collect();
        try_align_floating(
            &mut anchor_right,
            &lines,
            block,
            &right_anchored,
            4.0,
            |item| item.x + item.width,
            |item| anchor_key(item.x + item.width),
        );
        let center_anchored: HashSet<(usize, usize)> = anchor_center
            .values()
            .flat_map(|v| v.iter().copied())
            .collect();
        try_align_floating(
            &mut anchor_center,
            &lines,
            block,
            &center_anchored,
            4.0,
            |item| item.x + item.width * 0.5,
            |item| anchor_key(item.x + item.width * 0.5),
        );

        // Remove singletons
        anchor_left.retain(|_, v| v.len() >= 2);
        anchor_right.retain(|_, v| v.len() >= 2);
        anchor_center.retain(|_, v| v.len() >= 2);

        // --- Flowing block detection ---
        if is_flowing_text_block(
            &lines,
            block,
            &anchor_left,
            &anchor_right,
            &anchor_center,
            page.page_width,
            median_width,
        ) {
            render_flowing_block(&mut lines, block, &mut raw_lines, &mut meta, median_width);
            continue;
        }

        // --- Assign anchors to items in this block ---
        for (anchor, members) in &anchor_left {
            for &(li, bi) in members {
                meta[li][bi].left_anchor = Some(*anchor);
            }
        }
        for (anchor, members) in &anchor_right {
            for &(li, bi) in members {
                meta[li][bi].right_anchor = Some(*anchor);
            }
        }
        for (anchor, members) in &anchor_center {
            for &(li, bi) in members {
                meta[li][bi].center_anchor = Some(*anchor);
            }
        }

        // Resolve snap kind (strongest anchor wins; tie-break: left > right > center)
        for meta_line in meta.iter_mut().take(block.end).skip(block.start) {
            for m in meta_line {
                let left_count = m
                    .left_anchor
                    .and_then(|k| anchor_left.get(&k).map(|v| v.len()))
                    .unwrap_or(0);
                let right_count = m
                    .right_anchor
                    .and_then(|k| anchor_right.get(&k).map(|v| v.len()))
                    .unwrap_or(0);
                let center_count = m
                    .center_anchor
                    .and_then(|k| anchor_center.get(&k).map(|v| v.len()))
                    .unwrap_or(0);

                if left_count == 0 && right_count == 0 && center_count == 0 {
                    continue;
                }

                // Prefer left alignment when left and right counts are close.
                // In justified text, the right margin often collects slightly more
                // members than the left (due to indented paragraphs breaking the
                // left margin but keeping the right). A left-bias prevents justified
                // body text from right-snapping, which causes ragged left margins.
                let left_biased = left_count > 0
                    && right_count > 0
                    && left_count as f64 >= right_count as f64 * 0.8;

                let kind =
                    if (left_count >= right_count || left_biased) && left_count >= center_count {
                        SnapKind::Left
                    } else if right_count >= left_count && right_count >= center_count {
                        SnapKind::Right
                    } else {
                        SnapKind::Center
                    };
                m.snap = Some(kind);
            }
        }

        // Fixup pass: In justified text columns, most items share both a left
        // and right anchor. Items that lost their left anchor (e.g., indented
        // first lines) get right-snapped, but they should render at their
        // natural position. If a right anchor's members are overwhelmingly
        // also left-anchored, right-only members are likely justified body text
        // and should be unsnapped (rendered floating at natural x).
        //
        // Additional guard: the item's left x must be near the left x of the
        // left-anchored members (within half a page width). This prevents
        // unsnapping items in a different column that happen to share a right margin.
        {
            // For each right anchor: (total, has_left, median_left_x of left-anchored members)
            let mut right_anchor_info: HashMap<i32, (usize, usize, f32)> = HashMap::new();
            for (anchor_key, members) in &anchor_right {
                let total = members.len();
                let mut left_xs: Vec<f32> = Vec::new();
                let mut has_left = 0usize;
                for &(li, bi) in members {
                    if meta[li][bi].left_anchor.is_some() {
                        has_left += 1;
                        left_xs.push(lines[li][bi].item.x);
                    }
                }
                left_xs.sort_by(|a, b| a.total_cmp(b));
                let median_x = if left_xs.is_empty() {
                    0.0
                } else {
                    left_xs[left_xs.len() / 2]
                };
                right_anchor_info.insert(*anchor_key, (total, has_left, median_x));
            }

            for line_idx in block.start..block.end {
                for (box_idx, m) in meta[line_idx].iter_mut().enumerate() {
                    if m.snap != Some(SnapKind::Right) {
                        continue;
                    }
                    // Only fix items with no left anchor
                    if m.left_anchor.is_some() {
                        continue;
                    }
                    let Some(right_key) = m.right_anchor else {
                        continue;
                    };
                    let (total, has_left, median_left_x) = right_anchor_info
                        .get(&right_key)
                        .copied()
                        .unwrap_or((0, 0, 0.0));
                    // Require overwhelming majority (90%+) of members to have left anchors
                    if total < 10 || (has_left as f64 / total as f64) < 0.9 {
                        continue;
                    }
                    // Check that the item is near the same column as the left-anchored members
                    let item_x = lines[line_idx][box_idx].item.x;
                    let x_distance = (item_x - median_left_x).abs();
                    if x_distance > page.page_width * 0.25 {
                        continue;
                    }
                    if debug {
                        let preview: String = lines[line_idx]
                            .get(box_idx)
                            .map(|t| t.item.text.chars().take(30).collect())
                            .unwrap_or_default();
                        eprintln!(
                            "[debug] FIXUP unsnap: line={} right_anchor={} total={} has_left={} median_x={:.1} item_x={:.1} text='{}'",
                            line_idx, right_key, total, has_left, median_left_x, item_x, preview
                        );
                    }
                    m.snap = None;
                    m.right_anchor = None;
                }
            }
        }

        // Symmetric fixup: In multi-column layouts, short right-column lines
        // may only have a left anchor (no right anchor) because their right
        // edge doesn't align with full-width lines. Meanwhile, the full-width
        // lines in the same left anchor group get right-snapped. The short
        // lines get left-snapped and placed too far left.
        //
        // Detection: if a left anchor has many right-snapped members AND the
        // anchor position is past the page midpoint (indicating right column),
        // unsnap the remaining left-only members so they float and inherit
        // correct positioning from forward anchors.
        {
            let page_mid_key = anchor_key(page.page_width * 0.4);

            let mut left_anchor_info: HashMap<i32, (usize, usize)> = HashMap::new();
            for (left_ak, members) in &anchor_left {
                let total = members.len();
                let mut right_snapped = 0usize;
                for &(li, bi) in members {
                    if meta[li][bi].snap == Some(SnapKind::Right) {
                        right_snapped += 1;
                    }
                }
                left_anchor_info.insert(*left_ak, (total, right_snapped));
            }

            for line_idx in block.start..block.end {
                for (box_idx, m) in meta[line_idx].iter_mut().enumerate() {
                    if m.snap != Some(SnapKind::Left) {
                        continue;
                    }
                    // Only fix items with no right anchor (short lines)
                    if m.right_anchor.is_some() {
                        continue;
                    }
                    let Some(left_key) = m.left_anchor else {
                        continue;
                    };
                    // Only apply to items past the page midpoint (right column)
                    if left_key < page_mid_key {
                        continue;
                    }
                    let (total, right_snapped) =
                        left_anchor_info.get(&left_key).copied().unwrap_or((0, 0));
                    // Require a meaningful group where most members are right-snapped
                    if total < 4 || (right_snapped as f64 / total as f64) < 0.5 {
                        continue;
                    }
                    if debug {
                        let preview: String = lines[line_idx]
                            .get(box_idx)
                            .map(|t| t.item.text.chars().take(30).collect())
                            .unwrap_or_default();
                        eprintln!(
                            "[debug] FIXUP left-unsnap: line={} left_anchor={} total={} right_snapped={} text='{}'",
                            line_idx, left_key, total, right_snapped, preview
                        );
                    }
                    m.snap = None;
                    m.left_anchor = None;
                    m.force_unsnapped = true;
                }
            }
        }

        // --- Per-line flowing detection (within structured blocks) ---
        detect_and_render_flowing_lines(
            &mut lines,
            block,
            &mut raw_lines,
            &mut meta,
            median_width,
            page.page_width,
        );

        // --- Compute spacing hints (skip already-rendered flowing items) ---
        for line_idx in block.start..block.end {
            for box_idx in 0..lines[line_idx].len() {
                if meta[line_idx][box_idx].rendered {
                    continue;
                }
                if box_idx == 0 || meta[line_idx][box_idx - 1].rendered {
                    meta[line_idx][box_idx].should_space = 0;
                    continue;
                }
                let prev = &lines[line_idx][box_idx - 1].item;
                let cur = &lines[line_idx][box_idx].item;
                let x_delta = cur.x - (prev.x + prev.width);

                let mut should_space = 0usize;
                if x_delta > 2.0 {
                    should_space = 1;
                    let prev_len = prev.text.chars().count().max(1) as f32;
                    let prev_char_width = (prev.width / prev_len).max(0.1);
                    if x_delta > prev_char_width * 2.0 {
                        let column_gap_threshold = page.page_width * 0.1;
                        // Also detect column gaps using the relative gap method:
                        // if this line has a gap that's an outlier compared to other gaps,
                        // it's a column separator even if below the absolute threshold.
                        let has_relative_column_gap =
                            line_has_column_gap(&lines[line_idx], median_width, page.page_width);
                        let same_column = x_delta < column_gap_threshold
                            && !(has_relative_column_gap && x_delta > median_width * 2.0);

                        let cur_snap = meta[line_idx][box_idx].snap;
                        let prev_snap = meta[line_idx][box_idx - 1].snap;
                        let cur_is_left_snap = cur_snap == Some(SnapKind::Left);
                        let prev_is_right_snap = prev_snap == Some(SnapKind::Right);
                        let both_snapped = cur_snap.is_some() && prev_snap.is_some();
                        let force_unsnapped = meta[line_idx][box_idx].force_unsnapped;

                        if (!force_unsnapped && x_delta > prev_char_width * 8.0)
                            || cur_is_left_snap
                            || prev_is_right_snap
                            || both_snapped
                        {
                            should_space = if same_column {
                                FLOATING_SPACES
                            } else {
                                COLUMN_SPACES
                            };
                        } else {
                            should_space = if same_column { 1 } else { FLOATING_SPACES };
                        }
                    }
                }
                meta[line_idx][box_idx].should_space = should_space;
            }
        }

        // --- Build block-scoped snap lists ---
        let mut left_snaps: Vec<i32> = anchor_left.keys().copied().collect();
        let mut right_snaps: Vec<i32> = anchor_right.keys().copied().collect();
        let mut center_snaps: Vec<i32> = anchor_center.keys().copied().collect();
        left_snaps.sort_unstable();
        right_snaps.sort_unstable();
        center_snaps.sort_unstable();

        let mut floating_set: HashSet<i32> = HashSet::new();
        for line_idx in block.start..block.end {
            for (box_idx, bbox) in lines[line_idx].iter().enumerate() {
                if meta[line_idx][box_idx].snap.is_none() && !meta[line_idx][box_idx].rendered {
                    floating_set.insert(anchor_key(bbox.item.x));
                }
            }
        }
        let mut floating_snaps: Vec<i32> = floating_set.into_iter().collect();
        floating_snaps.sort_unstable();

        // --- Main rendering loop (scoped to block) ---
        let mut has_changed = true;
        while has_changed
            || !left_snaps.is_empty()
            || !right_snaps.is_empty()
            || !center_snaps.is_empty()
        {
            has_changed = false;

            // Render floating/unsnapped items first
            for line_idx in block.start..block.end {
                for box_idx in 0..lines[line_idx].len() {
                    if meta[line_idx][box_idx].rendered {
                        continue;
                    }

                    if !meta[line_idx][box_idx].force_unsnapped {
                        if meta[line_idx][box_idx].snap.is_some() {
                            continue;
                        }

                        let x_key = anchor_key(lines[line_idx][box_idx].item.x);
                        let center_key = anchor_key(
                            lines[line_idx][box_idx].item.x
                                + lines[line_idx][box_idx].item.width * 0.5,
                        );
                        if left_snaps.first().copied().is_some_and(|v| v < x_key)
                            || right_snaps.first().copied().is_some_and(|v| v < x_key)
                            || center_snaps
                                .first()
                                .copied()
                                .is_some_and(|v| v < center_key)
                        {
                            continue;
                        }
                    } else {
                        // Force-unsnapped items (e.g. short right-column lines)
                        // must wait until ALL snaps are processed so that forward
                        // anchors from their column neighbors are established.
                        if !left_snaps.is_empty()
                            || !right_snaps.is_empty()
                            || !center_snaps.is_empty()
                        {
                            continue;
                        }
                    }

                    if !can_render_bbox(&meta[line_idx], box_idx) {
                        break;
                    }

                    let (bbox_x, bbox_w, bbox_text) = {
                        let b = &lines[line_idx][box_idx].item;
                        (b.x, b.width, b.text.clone())
                    };
                    let mut target_x = ((bbox_x / median_width).round() as isize)
                        .max(0)
                        .min(COLUMN_SPACES as isize)
                        as usize;

                    let x_key = anchor_key(bbox_x);
                    let last_snap_left = forward_left
                        .range(..=x_key)
                        .map(|(_, v)| *v)
                        .max()
                        .unwrap_or(0);

                    let line_max = last_snap_left.max(
                        trim_end_len(&raw_lines[line_idx]) + meta[line_idx][box_idx].should_space,
                    );
                    if target_x < line_max {
                        target_x = line_max;
                    }

                    if !meta[line_idx][box_idx].force_unsnapped {
                        let floating_key = anchor_key(bbox_x);
                        if let Some(floating_anchor) = forward_floating.get(&floating_key).copied()
                            && target_x < floating_anchor
                        {
                            let adjusted = floating_anchor.min(target_x + 4);
                            if adjusted > target_x {
                                target_x = adjusted;
                            }
                        }
                    }

                    if debug && bbox_text.contains("Translation") {
                        eprintln!(
                            "[debug] FLOATING render '{bbox_text:.30}' line={line_idx} target_x={target_x} last_snap_left={} raw_len={} should_space={}",
                            forward_left
                                .range(..=anchor_key(bbox_x))
                                .map(|(_, v)| *v)
                                .max()
                                .unwrap_or(0),
                            char_len(&raw_lines[line_idx]),
                            meta[line_idx][box_idx].should_space
                        );
                        eprintln!("[debug]   raw_line so far: '{}'", &raw_lines[line_idx]);
                    }

                    trim_end_in_place(&mut raw_lines[line_idx]);
                    let before_len = char_len(&raw_lines[line_idx]);
                    if target_x > before_len {
                        raw_lines[line_idx].push_str(&" ".repeat(target_x - before_len));
                    }
                    let start_x = char_len(&raw_lines[line_idx]);
                    raw_lines[line_idx].push_str(&bbox_text);

                    meta[line_idx][box_idx].rendered = true;
                    meta[line_idx][box_idx].projected_x = start_x;
                    lines[line_idx][box_idx].rendered = true;
                    lines[line_idx][box_idx].num_spaces = start_x.saturating_sub(before_len);
                    has_changed = true;

                    let next_should_space = if box_idx + 1 < lines[line_idx].len() {
                        meta[line_idx][box_idx + 1].should_space
                    } else {
                        0
                    };
                    let right_bound = anchor_key(bbox_x + bbox_w);
                    let target_len = char_len(&raw_lines[line_idx]) + next_should_space;

                    update_forward_anchor_right_bound(
                        &left_snaps,
                        &mut forward_left,
                        right_bound,
                        target_len,
                    );
                    update_forward_anchor_right_bound(
                        &right_snaps,
                        &mut forward_right,
                        right_bound,
                        target_len,
                    );
                    update_forward_anchor_right_bound(
                        &floating_snaps,
                        &mut forward_floating,
                        right_bound,
                        target_len,
                    );
                }
            }

            // Pick next snap to process
            let left_first = left_snaps.first().copied();
            let right_first = right_snaps.first().copied();
            let center_first = center_snaps.first().copied();

            let next_kind = match (left_first, right_first, center_first) {
                (None, None, None) => None,
                (Some(_), None, None) => Some(SnapKind::Left),
                (None, Some(_), None) => Some(SnapKind::Right),
                (None, None, Some(_)) => Some(SnapKind::Center),
                (Some(l), Some(r), None) => Some(if l <= r {
                    SnapKind::Left
                } else {
                    SnapKind::Right
                }),
                (Some(l), None, Some(c)) => Some(if l <= c {
                    SnapKind::Left
                } else {
                    SnapKind::Center
                }),
                (None, Some(r), Some(c)) => Some(if r <= c {
                    SnapKind::Right
                } else {
                    SnapKind::Center
                }),
                (Some(l), Some(r), Some(c)) => {
                    if l <= r && l <= c {
                        Some(SnapKind::Left)
                    } else if r <= l && r <= c {
                        Some(SnapKind::Right)
                    } else {
                        Some(SnapKind::Center)
                    }
                }
            };

            let Some(kind) = next_kind else {
                continue;
            };

            let current_anchor = match kind {
                SnapKind::Left => left_snaps.first().copied(),
                SnapKind::Right => right_snaps.first().copied(),
                SnapKind::Center => center_snaps.first().copied(),
            };

            let Some(current_anchor) = current_anchor else {
                continue;
            };

            // Find items in this block matching the current anchor
            let mut turn_items: Vec<(usize, usize)> = Vec::new();
            for line_idx in block.start..block.end {
                for (box_idx, m) in meta[line_idx]
                    .iter()
                    .enumerate()
                    .take(lines[line_idx].len())
                {
                    if m.rendered {
                        continue;
                    }
                    let matches = match kind {
                        SnapKind::Left => {
                            m.left_anchor == Some(current_anchor)
                                && m.snap != Some(SnapKind::Right)
                                && m.snap != Some(SnapKind::Center)
                        }
                        SnapKind::Right => {
                            m.right_anchor == Some(current_anchor)
                                && m.snap == Some(SnapKind::Right)
                        }
                        SnapKind::Center => {
                            m.center_anchor == Some(current_anchor)
                                && m.snap == Some(SnapKind::Center)
                        }
                    };
                    if matches {
                        turn_items.push((line_idx, box_idx));
                    }
                }
            }

            if turn_items.is_empty() {
                match kind {
                    SnapKind::Left => {
                        left_snaps.remove(0);
                    }
                    SnapKind::Right => {
                        right_snaps.remove(0);
                    }
                    SnapKind::Center => {
                        center_snaps.remove(0);
                    }
                }
                continue;
            }

            has_changed = true;

            let mut target_x = ((anchor_to_x(current_anchor) / median_width).round() as isize)
                .max(0)
                .min(COLUMN_SPACES as isize) as usize;

            let line_max = match kind {
                SnapKind::Left => turn_items
                    .iter()
                    .map(|(li, bi)| {
                        char_len(&raw_lines[*li])
                            + line_space_end(&raw_lines[*li], meta[*li][*bi].should_space)
                            + 1
                    })
                    .max()
                    .unwrap_or(0),
                SnapKind::Right => turn_items
                    .iter()
                    .map(|(li, bi)| {
                        let bbox = &lines[*li][*bi].item;
                        let x_key = anchor_key(bbox.x);
                        let last_snap_left = forward_left
                            .range(..=x_key)
                            .map(|(_, v)| *v)
                            .max()
                            .unwrap_or(0);
                        let left_bound = last_snap_left
                            .max(trim_end_len(&raw_lines[*li]) + meta[*li][*bi].should_space);
                        left_bound + bbox.text.chars().count()
                    })
                    .max()
                    .unwrap_or(0),
                SnapKind::Center => turn_items
                    .iter()
                    .map(|(li, bi)| {
                        let text_half = lines[*li][*bi].item.text.chars().count() / 2;
                        char_len(&raw_lines[*li])
                            + text_half
                            + line_space_end(&raw_lines[*li], meta[*li][*bi].should_space)
                    })
                    .max()
                    .unwrap_or(0),
            };

            if target_x < line_max {
                target_x = line_max;
            }

            match kind {
                SnapKind::Left => {
                    if let Some(v) = forward_left.get(&current_anchor).copied() {
                        target_x = target_x.max(v);
                    }
                    forward_left.insert(current_anchor, target_x);
                }
                SnapKind::Right => {
                    if let Some(v) = forward_right.get(&current_anchor).copied() {
                        target_x = target_x.max(v);
                    }
                    forward_right.insert(current_anchor, target_x);
                }
                SnapKind::Center => {
                    if let Some(v) = forward_center.get(&current_anchor).copied() {
                        target_x = target_x.max(v);
                    }
                    forward_center.insert(current_anchor, target_x);
                }
            }

            if debug {
                let sample_text: Vec<String> = turn_items
                    .iter()
                    .take(2)
                    .map(|(li, bi)| {
                        let t = &lines[*li][*bi].item.text;
                        format!("'{:.30}'", t)
                    })
                    .collect();
                eprintln!(
                    "[debug] SNAP {:?} anchor={current_anchor} target_x={target_x} line_max={line_max} items={} samples={}",
                    kind,
                    turn_items.len(),
                    sample_text.join(", ")
                );
            }

            for (line_idx, box_idx) in turn_items {
                let (bbox_x, bbox_w, bbox_text) = {
                    let b = &lines[line_idx][box_idx].item;
                    (b.x, b.width, b.text.clone())
                };
                match kind {
                    SnapKind::Left => {
                        let before = char_len(&raw_lines[line_idx]);
                        if target_x > before {
                            raw_lines[line_idx].push_str(&" ".repeat(target_x - before));
                        }
                        let start_x = char_len(&raw_lines[line_idx]);
                        raw_lines[line_idx].push_str(&bbox_text);
                        meta[line_idx][box_idx].projected_x = start_x;
                        lines[line_idx][box_idx].num_spaces = start_x.saturating_sub(before);
                    }
                    SnapKind::Right => {
                        trim_end_in_place(&mut raw_lines[line_idx]);
                        let text_len = bbox_text.chars().count();
                        let before = char_len(&raw_lines[line_idx]);
                        let trim_len = trim_end_len(&raw_lines[line_idx]);
                        if target_x > trim_len + text_len {
                            let pad = target_x - char_len(&raw_lines[line_idx]) - text_len;
                            raw_lines[line_idx].push_str(&" ".repeat(pad));
                        }
                        let start_x = char_len(&raw_lines[line_idx]);
                        raw_lines[line_idx].push_str(&bbox_text);
                        meta[line_idx][box_idx].projected_x = start_x;
                        lines[line_idx][box_idx].num_spaces = start_x.saturating_sub(before);
                    }
                    SnapKind::Center => {
                        let text_half = bbox_text.chars().count() / 2;
                        let before = char_len(&raw_lines[line_idx]);
                        if target_x > char_len(&raw_lines[line_idx]) + text_half {
                            let pad = target_x - char_len(&raw_lines[line_idx]) - text_half;
                            raw_lines[line_idx].push_str(&" ".repeat(pad));
                        }
                        let start_x = char_len(&raw_lines[line_idx]);
                        raw_lines[line_idx].push_str(&bbox_text);
                        meta[line_idx][box_idx].projected_x = start_x;
                        lines[line_idx][box_idx].num_spaces = start_x.saturating_sub(before);
                    }
                }

                meta[line_idx][box_idx].rendered = true;
                lines[line_idx][box_idx].rendered = true;

                let next_should_space = if box_idx + 1 < lines[line_idx].len() {
                    meta[line_idx][box_idx + 1].should_space
                } else {
                    0
                };
                let right_bound = anchor_key(bbox_x + bbox_w);
                let target_len = char_len(&raw_lines[line_idx]) + next_should_space;
                update_forward_anchor_right_bound(
                    &left_snaps,
                    &mut forward_left,
                    right_bound,
                    target_len,
                );
                update_forward_anchor_right_bound(
                    &right_snaps,
                    &mut forward_right,
                    right_bound,
                    target_len,
                );
                update_forward_anchor_right_bound(
                    &floating_snaps,
                    &mut forward_floating,
                    right_bound,
                    target_len,
                );
            }

            match kind {
                SnapKind::Left => {
                    left_snaps.remove(0);
                }
                SnapKind::Right => {
                    right_snaps.remove(0);
                }
                SnapKind::Center => {
                    center_snaps.remove(0);
                }
            }
        }

        // Fallback: render anything still not rendered in this block
        for line_idx in block.start..block.end {
            for box_idx in 0..lines[line_idx].len() {
                if meta[line_idx][box_idx].rendered {
                    continue;
                }
                if !raw_lines[line_idx].is_empty() && !raw_lines[line_idx].ends_with(' ') {
                    raw_lines[line_idx].push(' ');
                }
                let start_x = char_len(&raw_lines[line_idx]);
                raw_lines[line_idx].push_str(&lines[line_idx][box_idx].item.text);
                meta[line_idx][box_idx].rendered = true;
                meta[line_idx][box_idx].projected_x = start_x;
                lines[line_idx][box_idx].rendered = true;
            }
        }
    }

    // Fixup: align rotated floating items with snapped items on adjacent lines.
    // Rotated labels (e.g. pin names) are excluded from anchors and render at
    // their natural x/median_width position.  If a neighboring line has snapped
    // content at a similar PDF x but a different column, shift the rotated items
    // to match, keeping relative spacing intact.
    for block in &blocks {
        for line_idx in block.start..block.end {
            // Collect rotated floating items on this line
            let rotated_items: Vec<usize> = (0..meta[line_idx].len())
                .filter(|&bi| {
                    bi < lines[line_idx].len()
                        && lines[line_idx][bi].rotated
                        && meta[line_idx][bi].snap.is_none()
                })
                .collect();
            if rotated_items.is_empty() {
                continue;
            }

            // Find the first rotated item's PDF x and rendered column
            let first_bi = rotated_items[0];
            let rot_pdf_x = lines[line_idx][first_bi].item.x;
            let rot_col = meta[line_idx][first_bi].projected_x;

            // Scan nearby lines (up to 4 away) for left/right-snapped items
            // at a similar PDF x. Prefer non-center snaps since center-snapped
            // items can be pulled out of position by unrelated anchor groups.
            {
                let scan_range = 4usize;
                let lo = if line_idx >= block.start + scan_range {
                    line_idx - scan_range
                } else {
                    block.start
                };
                let hi = (line_idx + scan_range + 1).min(block.end);

                let mut best: Option<(f32, usize, bool)> = None; // (x_diff, col, is_center)
                for adj in lo..hi {
                    if adj == line_idx {
                        continue;
                    }
                    for (bi, m) in meta[adj].iter().enumerate() {
                        if bi >= lines[adj].len() || m.snap.is_none() {
                            continue;
                        }
                        let is_center = m.snap == Some(SnapKind::Center);
                        let adj_pdf_x = lines[adj][bi].item.x;
                        let x_diff = (adj_pdf_x - rot_pdf_x).abs();
                        if x_diff < median_width * 3.0 {
                            let dominated = if let Some((prev_diff, _, prev_center)) = best {
                                // Prefer non-center over center; among same type prefer closer x
                                if !is_center && prev_center {
                                    true
                                } else if is_center && !prev_center {
                                    false
                                } else {
                                    x_diff < prev_diff
                                }
                            } else {
                                true
                            };
                            if dominated {
                                best = Some((x_diff, m.projected_x, is_center));
                            }
                        }
                    }
                }
                let best = best.map(|(d, c, _)| (d, c));

                if let Some((_, adj_col)) = best
                    && adj_col < rot_col
                {
                    let shift = rot_col - adj_col;
                    // Rebuild the line: shift all rotated items left
                    let first_col = meta[line_idx][first_bi].projected_x;
                    let pre = &raw_lines[line_idx][..raw_lines[line_idx]
                        .char_indices()
                        .nth(first_col)
                        .map(|(i, _)| i)
                        .unwrap_or(raw_lines[line_idx].len())];
                    let pre_trimmed = pre.trim_end();
                    let mut new_line = pre_trimmed.to_string();

                    for &bi in &rotated_items {
                        let old_col = meta[line_idx][bi].projected_x;
                        let new_col = old_col.saturating_sub(shift);
                        let cur_len = char_len(&new_line);
                        if new_col > cur_len {
                            new_line.push_str(&" ".repeat(new_col - cur_len));
                        }
                        let start = char_len(&new_line);
                        new_line.push_str(&lines[line_idx][bi].item.text);
                        meta[line_idx][bi].projected_x = start;
                    }
                    raw_lines[line_idx] = new_line;

                    // Also shift center-snapped items on immediately adjacent
                    // lines whose PDF x is close to the rotated items' x range.
                    let rot_x_min = rotated_items
                        .iter()
                        .map(|&bi| lines[line_idx][bi].item.x)
                        .fold(f32::INFINITY, f32::min);
                    let rot_x_max = rotated_items
                        .iter()
                        .map(|&bi| {
                            let b = &lines[line_idx][bi].item;
                            b.x + b.width
                        })
                        .fold(f32::NEG_INFINITY, f32::max);

                    let immediate_adj: Vec<usize> = [
                        line_idx.checked_sub(1).filter(|&l| l >= block.start),
                        Some(line_idx + 1).filter(|&l| l < block.end),
                    ]
                    .into_iter()
                    .flatten()
                    .collect();

                    // Also shift center-snapped items on immediately adjacent
                    // lines whose PDF x overlaps the rotated items' x range.
                    // These items share the same spatial region but may have been
                    // pulled out of position by center-snap constraints.
                    let corrected_first_col = meta[line_idx][first_bi].projected_x;

                    for adj in immediate_adj {
                        for bi in 0..lines[adj].len() {
                            if meta[adj][bi].snap != Some(SnapKind::Center) {
                                continue;
                            }
                            let b = &lines[adj][bi].item;
                            if b.x + b.width < rot_x_min - median_width * 2.0
                                || b.x > rot_x_max + median_width * 2.0
                            {
                                continue;
                            }
                            let old_col = meta[adj][bi].projected_x;
                            // Compute where this item should start based on
                            // the PDF x offset from the first rotated item.
                            let x_offset = b.x - lines[line_idx][first_bi].item.x;
                            let col_offset = (x_offset / median_width).round().max(0.0) as usize;
                            let target_col = corrected_first_col + col_offset;
                            if target_col > old_col {
                                let byte_start = raw_lines[adj]
                                    .char_indices()
                                    .nth(old_col)
                                    .map(|(i, _)| i)
                                    .unwrap_or(raw_lines[adj].len());
                                let pre = raw_lines[adj][..byte_start].trim_end().to_string();
                                let post_text = &raw_lines[adj][byte_start..];
                                let post_trimmed = post_text.trim_end();
                                let mut rebuilt = pre;
                                if target_col > char_len(&rebuilt) {
                                    rebuilt.push_str(&" ".repeat(target_col - char_len(&rebuilt)));
                                }
                                let start = char_len(&rebuilt);
                                rebuilt.push_str(post_trimmed);
                                raw_lines[adj] = rebuilt;
                                meta[adj][bi].projected_x = start;
                            }
                        }
                    }
                }
            }
        }
    }

    // Fix sparse blocks (per block)
    for block in &blocks {
        fix_sparse_blocks(&mut raw_lines, block.start, block.end);
    }

    // Persist projected positions and flatten in line order.
    let mut flattened: Vec<ProjectedTextItem> =
        Vec::with_capacity(lines.iter().map(|l| l.len()).sum());
    for (line_idx, line) in lines.into_iter().enumerate() {
        for (box_idx, mut item) in line.into_iter().enumerate() {
            item.force_unsnapped = meta[line_idx][box_idx].force_unsnapped;
            item.num_spaces = meta[line_idx][box_idx].should_space;

            if let Some(snap) = meta[line_idx][box_idx].snap {
                match snap {
                    SnapKind::Left => {
                        item.snap = Snap::Left;
                        item.anchor = Anchor::Left;
                    }
                    SnapKind::Right => {
                        item.snap = Snap::Right;
                        item.anchor = Anchor::Right;
                    }
                    SnapKind::Center => {
                        item.snap = Snap::Center;
                        item.anchor = Anchor::Center;
                    }
                }
            }
            flattened.push(item);
        }
    }

    clean_projected_items(&mut flattened, page.page_width);

    let text = raw_lines
        .into_iter()
        .map(|l| l.trim_end().to_string())
        .collect::<Vec<_>>()
        .join("\n");

    let text = clean_rendered_text(&text);

    (flattened, text)
}

/// Post-rendering text cleanup:
/// - Remove top margin (leading empty lines)
/// - Remove bottom margin (trailing empty lines)
/// - Remove left margin (consistent leading whitespace)
/// - Replace null characters with spaces
fn clean_rendered_text(text: &str) -> String {
    let text = text.replace('\0', " ");
    let lines: Vec<&str> = text.split('\n').collect();

    // Find bounds of content and minimum left indentation
    let mut min_x: Option<usize> = None;
    let mut min_y: Option<usize> = None;
    let mut max_y: Option<usize> = None;

    for (i, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let leading = line.len() - line.trim_start().len();
        min_x = Some(min_x.map_or(leading, |m: usize| m.min(leading)));
        if min_y.is_none() {
            min_y = Some(i);
        }
        max_y = Some(i);
    }

    let (min_x, min_y, max_y) = match (min_x, min_y, max_y) {
        (Some(x), Some(y1), Some(y2)) => (x, y1, y2),
        _ => return String::new(),
    };

    lines[min_y..=max_y]
        .iter()
        .map(|line| {
            if line.len() > min_x {
                &line[min_x..]
            } else {
                line.trim_end()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn project_pages_to_grid(pages: Vec<Page>) -> Vec<ParsedPage> {
    pages
        .into_iter()
        .map(|page| {
            let projection_boxes = page
                .text_items
                .iter()
                .map(|item| ProjectedTextItem {
                    orig_x: item.x,
                    orig_y: item.y,
                    orig_width: item.width,
                    orig_height: item.height,
                    orig_rotation: item.rotation,
                    item: item.clone(),
                    snap: Snap::Left,
                    anchor: Anchor::Left,
                    is_dup: false,
                    rendered: false,
                    num_spaces: 0,
                    force_unsnapped: false,
                    is_margin_line_number: false,
                    rotated: false,
                    d: 0.0,
                })
                .collect();

            let (projected_items, text) = project_to_grid(&page, projection_boxes);
            // Detect figure regions from the page's vector graphics before
            // XY-cut runs so the layout recursion can treat them as obstacles
            // (partition the page around figures rather than slicing through).
            let figures = crate::figure_cluster::detect_figure_rects(
                &page.graphics,
                &page.text_items,
                page.page_width,
                page.page_height,
            );
            // Pre-projection ruled-table detection. Feeds XY-cut as obstacles
            // so a table's inter-column gaps don't get picked as page-level
            // V-cuts (column-major reading order is the dominant TEDS=0
            // failure mode on the bench: docs 083/120/130/etc.).
            let table_rects = crate::markdown_layout::detect_table_rects(
                &page.graphics,
                page.page_width,
                page.page_height,
            );
            let mut obstacles: Vec<Rect> = Vec::with_capacity(figures.len() + table_rects.len());
            obstacles.extend(figures.iter().cloned());
            obstacles.extend(table_rects.iter().cloned());
            let (projected_lines, regions) = build_projected_lines(
                &projected_items,
                page.page_width,
                page.page_height,
                &obstacles,
            );
            ParsedPage {
                page_number: page.page_number,
                page_width: page.page_width,
                page_height: page.page_height,
                text,
                markdown: String::new(),
                text_items: projected_items
                    .into_iter()
                    .map(|proj| TextItem {
                        x: proj.orig_x,
                        y: proj.orig_y,
                        width: proj.orig_width,
                        height: proj.orig_height,
                        rotation: proj.orig_rotation,
                        ..proj.item
                    })
                    .collect(),
                projected_lines,
                regions,
                graphics: page.graphics,
                vector_graphics: page.vector_graphics,
                figures,
                struct_nodes: page.struct_nodes,
                image_refs: page.image_refs,
                complexity: None,
            }
        })
        .collect()
}

// ── ProjectedLine derivation ────────────────────────────────────────────────
//
// `build_projected_lines` groups already-projected items (in reading order)
// into per-line structural metadata consumed by the markdown emitter. Lines
// are grouped by y-proximity; aggregates (dominant font, all-bold/italic/mono)
// are char-weighted across the line's items. The JSON/text outputs are
// unaffected — `ParsedPage.projected_lines` is `#[serde(skip)]`.

/// PDF font descriptor flag bits we care about.
/// See PDF spec §9.8.2 (table 123).
const PDF_FONT_FLAG_FIXED_PITCH: i32 = 1; // bit 1
const PDF_FONT_FLAG_ITALIC: i32 = 64; // bit 7
const PDF_FONT_FLAG_FORCE_BOLD: i32 = 262144; // bit 19

/// Font-name substrings that indicate bold weight.
const BOLD_NAME_HINTS: &[&str] = &["Bold", "Black", "Heavy", "Semibold", "Demibold"];
/// Font-name substrings that indicate italic / oblique slant.
const ITALIC_NAME_HINTS: &[&str] = &["Italic", "Oblique"];
/// Font-name substrings that indicate a monospaced typeface.
const MONO_NAME_HINTS: &[&str] = &[
    "Courier",
    "Mono",
    "Consolas",
    "Menlo",
    "Source Code",
    "Inconsolata",
    "Hack",
    "Fira Code",
];

fn name_contains_any(name: &str, hints: &[&str]) -> bool {
    hints.iter().any(|h| name.contains(h))
}

pub(crate) fn is_bold_item(item: &TextItem) -> bool {
    if let Some(flags) = item.font_flags
        && flags & PDF_FONT_FLAG_FORCE_BOLD != 0
    {
        return true;
    }
    if let Some(w) = item.font_weight
        && w >= 600
    {
        return true;
    }
    if let Some(n) = &item.font_name
        && name_contains_any(n, BOLD_NAME_HINTS)
    {
        return true;
    }
    false
}

pub(crate) fn is_italic_item(item: &TextItem) -> bool {
    if let Some(flags) = item.font_flags
        && flags & PDF_FONT_FLAG_ITALIC != 0
    {
        return true;
    }
    if let Some(n) = &item.font_name
        && name_contains_any(n, ITALIC_NAME_HINTS)
    {
        return true;
    }
    false
}

pub(crate) fn is_strike_item(item: &TextItem) -> bool {
    item.strike
}

pub(crate) fn is_mono_item(item: &TextItem) -> bool {
    if let Some(flags) = item.font_flags
        && flags & PDF_FONT_FLAG_FIXED_PITCH != 0
    {
        return true;
    }
    if let Some(n) = &item.font_name
        && name_contains_any(n, MONO_NAME_HINTS)
    {
        return true;
    }
    false
}

// ── XY-cut layout decomposition ─────────────────────────────────────────────
//
// Replaces the prior flat column detector. Recursively splits the page along H
// or V axes at "valleys" — runs of low projection density wider than a minimum
// threshold. Pre-order traversal of the resulting tree gives reading order
// (top→bottom, left→right) including for nested layouts (banded splits with
// sub-columns).
//
// Threshold is min-density relative to the local content's median: a cut is
// valid if the projection density in the valley stays below T_DENS × median
// for at least MIN_VALLEY_PT continuous units. Auto-scales to page content so
// dense vs. sparse pages need no per-doc tuning. Conservative defaults — we'd
// rather under-cut (merge columns) than shred a paragraph with wide
// inter-word gaps.

/// Bucket size for the 1D density projection (points).
const XY_BUCKET_PT: f32 = 2.0;
/// Density threshold for a "valley", expressed as a fraction of the local
/// median non-zero bucket density.
const XY_T_DENS: f32 = 0.10;
/// Minimum vertical-cut (column gutter) valley width.
const XY_MIN_V_VALLEY_PT: f32 = 8.0;
/// Minimum horizontal-cut (row gap) valley width, expressed as a multiple of
/// the median line height.
const XY_MIN_H_VALLEY_FACTOR: f32 = 1.4;
/// Hard recursion cap.
// Backstop only — every cut still passes the banner/density/histogram gates.
// 6 starved pages with several stacked floats (tables + captions above a
// 2-col body): each banner peel burns a level, and the body band leafs out
// un-split at the cap.
const XY_MAX_DEPTH: u32 = 12;
/// Below this item count we stop trying to cut.
const XY_MIN_ITEMS_TO_CUT: usize = 4;
/// A horizontal cut must leave at least this many distinct text lines on each
/// side; otherwise we don't slice prose into sliver bands.
const XY_MIN_LINES_PER_H_SIDE: usize = 2;
/// Vertical-cut score must beat the horizontal score by this factor to be
/// chosen. Tie-breaks in favor of horizontal so pages are sliced into bands
/// before columns — matches natural reading order.
const XY_V_PREFERENCE_MARGIN: f32 = 1.1;

/// Bucket width used by the column-start histogram fallback. 5pt is finer
/// than typical inter-column gutters (8–20pt) and coarser than baseline
/// noise / kerning drift, so column edges land in distinct buckets.
const XY_COLUMN_BUCKET_PT: f32 = 5.0;
/// Minimum lines stacked at the same left-edge cluster for the cluster to
/// count as a column peak. A real column has many lines at one x (15+);
/// indented paragraph-starts and footnote markers produce a handful (3–6).
/// We set this above the typical "indented paragraph" count so those drop
/// out and only true column-left edges remain.
const XY_COLUMN_MIN_LINES_PER_PEAK: usize = 10;
/// Minimum horizontal distance between two column peaks, as a fraction of
/// `bbox.width`. Sub-paragraph indents and list nesting produce
/// closely-spaced peaks; a real column gutter is much further out.
const XY_COLUMN_MIN_GAP_FRACTION: f32 = 0.25;
/// Minimum fraction of items that must start at one of the two detected
/// column peaks. Real 2-column layouts cluster nearly every line at one of
/// two left edges; tables, lists, and scattered prose don't.
const XY_COLUMN_PEAK_DOMINANCE: f32 = 0.55;
/// Minimum smaller/larger ratio between the two column peaks' line counts.
/// Balanced 2-column layouts have ~equal line counts in each column; a
/// 90/10 split is almost certainly a single column with one off-x bullet
/// stack, not a real two-column structure.
const XY_COLUMN_PEAK_BALANCE_RATIO: f32 = 0.4;
/// Minimum per-column horizontal fill for an N-column (3+ peak) split.
/// Genuine N-column prose fills each inter-peak slot nearly edge-to-edge
/// with text; a data table leaves wide intra-row whitespace, so its cells
/// cover only a fraction of each slot. Gate the N-column split on the
/// least-filled column so we keep splitting prose but bail (→ table
/// detector) on sparse tabular layouts.
const XY_COLUMN_MIN_FILL: f32 = 0.55;

/// A peak whose band count is below this fraction of the most-banded column's
/// is treated as a figure-scatter "phantom" (axis tick labels, a point cloud
/// spread across x) rather than a real column. Real table columns have one
/// cell per row, so their band counts are comparable to their siblings; a
/// plot embedded in 2-column prose contributes a sparse, low-fill third peak
/// that would otherwise flip the whole region to `tabular` and suppress the
/// genuine column split. See the phantom-drop in `xy_find_column_cut`.
const XY_COLUMN_PHANTOM_BAND_FRACTION: f32 = 0.25;

#[derive(Debug, Clone, Copy)]
struct CutCandidate {
    axis: CutAxis,
    /// Coordinate of the split (y for horizontal cut, x for vertical).
    position: f32,
    score: f32,
}

/// Character-weighted contribution of an item to the projection. Headings get
/// roughly the same weight as the body text they're competing with, instead of
/// being amplified by their bbox area.
fn xy_item_weight(it: &TextItem) -> f32 {
    it.text.chars().count().max(1) as f32
}

fn xy_root_bbox(items: &[ProjectedTextItem], page_width: f32, page_height: f32) -> Rect {
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for it in items {
        if it.item.text.is_empty() {
            continue;
        }
        min_x = min_x.min(it.item.x);
        min_y = min_y.min(it.item.y);
        max_x = max_x.max(it.item.x + it.item.width.max(0.0));
        max_y = max_y.max(it.item.y + it.item.height.max(0.0));
    }
    if !min_x.is_finite() {
        return Rect {
            x: 0.0,
            y: 0.0,
            width: page_width.max(1.0),
            height: page_height.max(1.0),
        };
    }
    let x = min_x.max(0.0);
    let y = min_y.max(0.0);
    let w = (max_x - x).max(1.0);
    let h = (max_y - y).max(1.0);
    // Span the actual item extent. The page dimensions are only used as a
    // fallback (no items). We must NOT clamp the bbox down to them:
    // `handle_rotation_reading_order` lays rotated marginalia (e.g. an arXiv
    // sidebar stamp) out on a virtual canvas taller/wider than the physical
    // page and pushes the non-rotated body below it. Clamping to page_height
    // would drop those displaced body items outside the density buckets, so no
    // horizontal valley between a full-width header and the 2-column body is
    // ever found and the column V-cut shreds the centered title/author band.
    Rect {
        x,
        y,
        width: w,
        height: h,
    }
}

fn xy_median_line_height(items: &[ProjectedTextItem], idxs: &[usize]) -> f32 {
    let mut heights: Vec<f32> = idxs
        .iter()
        .filter_map(|&i| {
            let h = items[i].item.height;
            if h > 0.5 { Some(h) } else { None }
        })
        .collect();
    if heights.is_empty() {
        return 10.0;
    }
    heights.sort_by(|a, b| a.total_cmp(b));
    heights[heights.len() / 2]
}

/// Approximate distinct-line count by y-banding, used only to enforce
/// `XY_MIN_LINES_PER_H_SIDE`.
fn xy_distinct_lines(items: &[ProjectedTextItem], idxs: &[usize], median_h: f32) -> usize {
    if idxs.is_empty() {
        return 0;
    }
    let band = (median_h * 0.5).max(2.0);
    let mut ys: Vec<f32> = idxs.iter().map(|&i| items[i].item.y).collect();
    ys.sort_by(|a, b| a.total_cmp(b));
    let mut lines = 1usize;
    let mut last = ys[0];
    for &y in &ys[1..] {
        if (y - last).abs() > band {
            lines += 1;
            last = y;
        }
    }
    lines
}

/// Find the best valley along `axis` inside `bbox` for the given items. The
/// returned position is in absolute page coordinates.
///
/// `figures` is the set of figure-region rectangles on the page. Any figure
/// that intersects `bbox` is stamped into the density projection like a giant
/// opaque text block — its perpendicular extent (clipped to `bbox`) is added
/// to each bucket it covers along `axis`. This makes the recursion prefer
/// cuts *around* figures rather than slicing through them (which is the
/// motivating fix for academic-paper page-1 layouts where a figure straddles
/// both columns).
fn xy_find_best_cut(
    items: &[ProjectedTextItem],
    idxs: &[usize],
    bbox: &Rect,
    axis: CutAxis,
    median_h: f32,
    figures: &[Rect],
) -> Option<CutCandidate> {
    let (origin, length) = match axis {
        CutAxis::Horizontal => (bbox.y, bbox.height),
        CutAxis::Vertical => (bbox.x, bbox.width),
    };
    if length <= 0.0 {
        return None;
    }
    let n_buckets = ((length / XY_BUCKET_PT).ceil() as usize).max(1);
    let mut density = vec![0.0f32; n_buckets];

    for &i in idxs {
        let it = &items[i].item;
        let (c0, c1) = match axis {
            CutAxis::Horizontal => (it.y, it.y + it.height.max(0.0)),
            CutAxis::Vertical => (it.x, it.x + it.width.max(0.0)),
        };
        let weight = xy_item_weight(it);
        let b0_f = ((c0 - origin) / XY_BUCKET_PT).floor();
        let b1_f = ((c1 - origin) / XY_BUCKET_PT).ceil();
        let b0 = b0_f.max(0.0) as usize;
        let b1 = (b1_f.max(b0_f + 1.0) as usize).min(n_buckets);
        for b in b0..b1 {
            density[b] += weight;
        }
    }

    // Stamp figures into the projection as opaque obstacles. A figure that
    // intersects `bbox` adds density to each axis-parallel bucket it spans;
    // weight = its perpendicular extent (clipped to bbox) so figures behave
    // like very wide/tall pseudo-items relative to text. The valley search
    // naturally finds cuts adjacent to figures rather than through them.
    for fig in figures {
        let fx0 = fig.x.max(bbox.x);
        let fy0 = fig.y.max(bbox.y);
        let fx1 = (fig.x + fig.width).min(bbox.x + bbox.width);
        let fy1 = (fig.y + fig.height).min(bbox.y + bbox.height);
        if fx1 <= fx0 || fy1 <= fy0 {
            continue;
        }
        let (c0, c1, perp) = match axis {
            CutAxis::Horizontal => (fy0, fy1, fx1 - fx0),
            CutAxis::Vertical => (fx0, fx1, fy1 - fy0),
        };
        let weight = perp.max(1.0);
        let b0_f = ((c0 - origin) / XY_BUCKET_PT).floor();
        let b1_f = ((c1 - origin) / XY_BUCKET_PT).ceil();
        let b0 = b0_f.max(0.0) as usize;
        let b1 = (b1_f.max(b0_f + 1.0) as usize).min(n_buckets);
        for b in b0..b1 {
            density[b] += weight;
        }
    }

    let mut nonzero: Vec<f32> = density.iter().copied().filter(|d| *d > 0.0).collect();
    if nonzero.is_empty() {
        return None;
    }
    nonzero.sort_by(|a, b| a.total_cmp(b));
    let median = nonzero[nonzero.len() / 2];
    if median <= 0.0 {
        return None;
    }
    let threshold = median * XY_T_DENS;

    // Don't cut at leading/trailing whitespace — only consider valleys interior
    // to the content's span on this axis.
    let first_dense = density.iter().position(|d| *d > threshold)?;
    let last_dense = density.iter().rposition(|d| *d > threshold)?;
    if last_dense <= first_dense + 1 {
        return None;
    }

    let min_valley_pt = match axis {
        CutAxis::Vertical => XY_MIN_V_VALLEY_PT,
        CutAxis::Horizontal => median_h * XY_MIN_H_VALLEY_FACTOR,
    };
    let min_valley_buckets = ((min_valley_pt / XY_BUCKET_PT).ceil() as usize).max(1);

    let mut best: Option<CutCandidate> = None;
    let mut i = first_dense + 1;
    while i <= last_dense {
        if density[i] <= threshold {
            let s = i;
            while i <= last_dense && density[i] <= threshold {
                i += 1;
            }
            let e = i; // exclusive
            let width = e - s;
            if width >= min_valley_buckets {
                let mean: f32 = density[s..e].iter().sum::<f32>() / width as f32;
                let depth = (1.0 - mean / median).max(0.0);
                let score = width as f32 * depth;
                let mid = (s as f32 + e as f32) * 0.5;
                let position = origin + mid * XY_BUCKET_PT;
                // Density-stamping alone isn't always enough to push the
                // gutter inside a table region below threshold — the
                // inter-cell gaps win on tables with sparsely-filled cells
                // (docs 120/180/150). Hard-reject any cut line that passes
                // strictly through an obstacle rect intersecting `bbox`.
                if !cut_line_passes_through_obstacle(axis, position, bbox, figures)
                    && best.as_ref().is_none_or(|b| score > b.score)
                {
                    best = Some(CutCandidate {
                        axis,
                        position,
                        score,
                    });
                }
            }
        } else {
            i += 1;
        }
    }
    best
}

/// Hard-reject companion to the density-stamping in `xy_find_best_cut`.
/// Density-stamping makes the recursion *prefer* cuts around obstacles, but
/// sparsely-filled tables (docs 120/180/150) have inter-cell gaps that still
/// score better than the stamped obstacle band. Returns true iff the cut
/// line at `position` strictly passes through any obstacle that intersects
/// `bbox`.
fn cut_line_passes_through_obstacle(
    axis: CutAxis,
    position: f32,
    bbox: &Rect,
    obstacles: &[Rect],
) -> bool {
    // Tolerance: a cut whose position lands within ~1pt of an obstacle's
    // edge is allowed (the obstacle's edge is itself a natural cut line).
    const EDGE_TOLERANCE_PT: f32 = 1.0;
    for ob in obstacles {
        // Skip obstacles that don't intersect bbox at all.
        let ox0 = ob.x.max(bbox.x);
        let oy0 = ob.y.max(bbox.y);
        let ox1 = (ob.x + ob.width).min(bbox.x + bbox.width);
        let oy1 = (ob.y + ob.height).min(bbox.y + bbox.height);
        if ox1 <= ox0 || oy1 <= oy0 {
            continue;
        }
        let through = match axis {
            CutAxis::Vertical => {
                position > ox0 + EDGE_TOLERANCE_PT && position < ox1 - EDGE_TOLERANCE_PT
            }
            CutAxis::Horizontal => {
                position > oy0 + EDGE_TOLERANCE_PT && position < oy1 - EDGE_TOLERANCE_PT
            }
        };
        if through {
            return true;
        }
    }
    false
}

fn xy_split_bbox(bbox: &Rect, cut: &CutCandidate) -> (Rect, Rect) {
    match cut.axis {
        CutAxis::Horizontal => {
            let top_h = (cut.position - bbox.y).max(0.0);
            let top = Rect {
                x: bbox.x,
                y: bbox.y,
                width: bbox.width,
                height: top_h,
            };
            let bot = Rect {
                x: bbox.x,
                y: cut.position,
                width: bbox.width,
                height: (bbox.height - top_h).max(0.0),
            };
            (top, bot)
        }
        CutAxis::Vertical => {
            let left_w = (cut.position - bbox.x).max(0.0);
            let left = Rect {
                x: bbox.x,
                y: bbox.y,
                width: left_w,
                height: bbox.height,
            };
            let right = Rect {
                x: cut.position,
                y: bbox.y,
                width: (bbox.width - left_w).max(0.0),
                height: bbox.height,
            };
            (left, right)
        }
    }
}

/// Minimum fraction of region width a banner band must cover.
const XY_BANNER_WIDTH_FRACTION: f32 = 0.6;
/// Banner clearance gap above/below in multiples of median line height.
const XY_BANNER_CLEARANCE_FACTOR: f32 = 1.5;

/// Detect a "banner" y-band — one or more vertically-adjacent wide text bands
/// (≥ `XY_BANNER_WIDTH_FRACTION` of bbox width) flanked by a clear vertical
/// gap from the rest of the content. Returns an H-cut that isolates the banner
/// from neighboring content.
///
/// Motivating case: a full-width centered title + authors block above a 2-
/// column body where a figure straddles both columns. Density-based XY-cut
/// can't separate them because the figure prevents any clean V-cut and there's
/// no obvious H-valley between the banner and the body. The banner detector
/// runs first and force-cuts just below the banner stack, freeing the
/// recursion to find normal column gutters in the lower region.
///
/// Also helps mid-document section headings that span the full content width:
/// a wide centered "5 Conclusion" line above a 2-column body would otherwise
/// get pulled into one column.
/// Fallback column-gutter detector for cases where the density-based V-cut
/// search finds no valley — typically because a column-spanning element
/// (wide heading, figure, table) fills the gutter at one y-band and inflates
/// the projection at the gutter's x. Detects 2+ column structure by clustering
/// the left edges of "narrow" lines (lines that don't span the full region
/// width). When two clusters with ≥`XY_COLUMN_MIN_LINES_PER_PEAK` lines each
/// are separated by ≥`XY_COLUMN_MIN_GAP_FRACTION × bbox.width`, returns a
/// V-cut at the midpoint between them with a high score so it beats any
/// density-based cut except banner.
fn xy_find_column_cut(
    items: &[ProjectedTextItem],
    idxs: &[usize],
    bbox: &Rect,
    median_h: f32,
    tabular: &mut bool,
) -> Option<CutCandidate> {
    let dbg = std::env::var("LITEPARSE_DEBUG_XY").is_ok();
    macro_rules! reject {
        ($($t:tt)*) => {{
            if dbg { eprintln!("[xy col-reject] {}", format!($($t)*)); }
            return None;
        }};
    }
    if bbox.width <= 1.0 || idxs.len() < 8 {
        reject!("preflight: width={:.1} items={}", bbox.width, idxs.len());
    }

    // Identify "line-start" items: within each y-band, the first item of
    // each horizontal run is a line-start. PDFium routinely emits mid-line
    // word-cluster fragments as separate items; counting all of them in
    // the histogram denominator dilutes the column-dominance ratio so the
    // wide-side probe never trips after a footer-peel H-cut. A 2-column
    // line at the same baseline contributes TWO line-starts (one per
    // column) because the inter-column gutter exceeds `gutter_gap`, so we
    // don't collapse a band to its single leftmost item.
    let band_tol = (median_h * 0.5).max(2.0);
    let gutter_gap = median_h.max(4.0);

    let mut sorted: Vec<usize> = idxs.to_vec();
    sorted.sort_by(|&a, &b| {
        items[a]
            .item
            .y
            .total_cmp(&items[b].item.y)
            .then(items[a].item.x.total_cmp(&items[b].item.x))
    });

    let mut line_starts: Vec<f32> = Vec::with_capacity(idxs.len() / 2);
    // Items whose gap to the previous item falls just under `gutter_gap` but
    // clearly above word-gap range. A tight column gutter (e.g. ~9pt at
    // median_h=10) fails the strict test on justified lines whose left column
    // runs right up to the gutter; these get a second chance below if their x
    // aligns with a column peak that has strict evidence.
    let mut near_starts: Vec<f32> = Vec::new();
    let near_gap_floor = gutter_gap * 0.6;
    let mut k = 0;
    while k < sorted.len() {
        let band_y = items[sorted[k]].item.y;
        let mut j = k;
        while j < sorted.len() && (items[sorted[j]].item.y - band_y).abs() <= band_tol {
            j += 1;
        }
        let mut band_items: Vec<usize> = sorted[k..j].to_vec();
        band_items.sort_by(|&a, &b| items[a].item.x.total_cmp(&items[b].item.x));
        let mut prev_right = f32::NEG_INFINITY;
        for &idx in &band_items {
            let it = &items[idx].item;
            if it.x.is_finite() {
                let gap = it.x - prev_right;
                if gap > gutter_gap {
                    line_starts.push(it.x);
                } else if gap > near_gap_floor {
                    near_starts.push(it.x);
                }
            }
            let right = it.x + it.width.max(0.0);
            if right > prev_right {
                prev_right = right;
            }
        }
        k = j;
    }

    if line_starts.len() < 8 {
        reject!(
            "line_starts={} < 8 (items={})",
            line_starts.len(),
            idxs.len()
        );
    }

    // Histogram extent uses full item bounds so `XY_COLUMN_MIN_GAP_FRACTION`
    // calibration (fraction of layout width) is preserved. Only line-starts
    // contribute to the bucket counts and the dominance denominator.
    let mut min_x = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    for &i in idxs {
        let it = &items[i].item;
        if it.x.is_finite() {
            min_x = min_x.min(it.x);
            max_x = max_x.max(it.x + it.width.max(0.0));
        }
    }
    if !min_x.is_finite() || max_x <= min_x + 1.0 {
        reject!("degenerate extent min_x={min_x:.1} max_x={max_x:.1}");
    }
    let total_w = max_x - min_x;
    let bucket_pt = XY_COLUMN_BUCKET_PT;
    let n_buckets = ((total_w / bucket_pt).ceil() as usize).max(1);
    let mut hist = vec![0usize; n_buckets];
    for &x in &line_starts {
        let b_f = ((x - min_x) / bucket_pt).floor();
        let b = b_f.max(0.0) as usize;
        if b < n_buckets {
            hist[b] += 1;
        }
    }

    // Peak-aligned rescue for tight gutters: a near-gap start only counts
    // when it lands at (nearly) the same x as existing strict-evidence
    // starts. A column left edge aligns to sub-point precision across
    // bands; word-gap "rivers" in justified text jitter by several points —
    // a looser bucket-width match here let rivers stack onto stray strict
    // starts and manufacture a fake second column inside narrow justified
    // prose. Require near-exact agreement with at least 2 strict starts.
    let mut rescued = 0usize;
    let mut counted_starts: Vec<f32> = line_starts.clone();
    if !near_starts.is_empty() {
        const RESCUE_ALIGN_TOL_PT: f32 = 1.5;
        for &x in &near_starts {
            let support = line_starts
                .iter()
                .filter(|&&s| (s - x).abs() <= RESCUE_ALIGN_TOL_PT)
                .count();
            if support >= 2 {
                let b = (((x - min_x) / bucket_pt).floor()).max(0.0) as usize;
                if b < n_buckets {
                    hist[b] += 1;
                    rescued += 1;
                    counted_starts.push(x);
                }
            }
        }
        if dbg && rescued > 0 {
            eprintln!(
                "[xy col-rescue] {rescued} near-gap starts (gap in [{near_gap_floor:.1},{gutter_gap:.1}]) exactly aligned to >=2 strict starts"
            );
        }
    }
    let n_starts = line_starts.len() + rescued;

    // Cluster adjacent occupied buckets into peaks. A peak is a contiguous
    // run of buckets with ≥1 count, merged through gaps of ≤2 zero buckets
    // (~10pt) to absorb slight intra-column drift.
    let mut peaks: Vec<(f32, usize)> = Vec::new(); // (x_center, item_count)
    let mut i = 0;
    while i < n_buckets {
        if hist[i] == 0 {
            i += 1;
            continue;
        }
        let s = i;
        let mut total = 0usize;
        let mut last_nonzero = i;
        let mut j = i;
        while j < n_buckets {
            if hist[j] > 0 {
                total += hist[j];
                last_nonzero = j;
                j += 1;
            } else if j - last_nonzero <= 2 {
                j += 1;
            } else {
                break;
            }
        }
        let x_center = min_x + bucket_pt * (s as f32 + last_nonzero as f32 + 1.0) * 0.5;
        peaks.push((x_center, total));
        i = j;
    }

    // Keep only "strong" peaks. A real column edge is sat-stacked by many
    // lines at the same left x; spurious peaks (a stray indented quote, a
    // numbered marker, etc.) carry only a few lines.
    let strong_before = peaks.len();
    let raw_counts: Vec<(f32, usize)> = peaks.clone();
    // Adaptive minimum: scale with how many line-starts the band actually has.
    // The static `XY_COLUMN_MIN_LINES_PER_PEAK = 10` floor is meant to drop
    // indented-paragraph false peaks (a handful of lines) on a large region,
    // but on smaller regions it can drop legitimate column peaks too. For a
    // body region with ~36 lines split across 2 columns, each peak only
    // carries 5–6 line-starts after the histogram spreads them across the
    // page width — well under 10. Use `max(line_starts / 6, 4)`: still rules
    // out the tiny indent peaks, but a 2-column region with ~5 lines/peak
    // qualifies. Capped by `XY_COLUMN_MIN_LINES_PER_PEAK` so large regions
    // keep the stricter floor.
    let adaptive_min = (n_starts / 6).clamp(4, XY_COLUMN_MIN_LINES_PER_PEAK);
    peaks.retain(|(_, c)| *c >= adaptive_min);
    if peaks.len() < 2 {
        if dbg {
            let preview: Vec<String> = raw_counts
                .iter()
                .take(8)
                .map(|(x, c)| format!("({x:.0}:{c})"))
                .collect();
            eprintln!(
                "[xy col-reject] peaks<2 after min_per_peak={adaptive_min} (raw={strong_before} strong={}): {}",
                peaks.len(),
                preview.join(",")
            );
        }
        return None;
    }
    peaks.sort_by(|a, b| a.0.total_cmp(&b.0));

    // Tabular layouts produce 3+ strong peaks (one per cell column). Real
    // page columns have exactly 2 dominant peaks (left edge of col 1, left
    // edge of col 2) — even multi-paragraph indents are sub-peaks weak
    // enough to drop out under the min_per_peak filter. Bail when we see
    // more than 2 strong peaks so we don't slice tables.
    // Tabular layouts produce 3+ strong peaks with comparable counts. But a
    // real 2-column page can also produce 3+ peaks when a centered float
    // (caption, sub-heading stack, pull-quote) sits between the columns and
    // contributes line-starts at a mid x. Distinguish: if the top-2 peaks
    // by count dominate the remainder, keep them and drop the rest;
    // otherwise (top-3 are comparable → tabular) bail.
    if peaks.len() > 2 {
        let mut by_count = peaks.clone();
        by_count.sort_by(|a, b| b.1.cmp(&a.1));
        let third = by_count[2].1 as f32;
        let second = by_count[1].1 as f32;
        let largest = by_count[0].1 as f32;
        if second <= 0.0 || third / second >= XY_COLUMN_PEAK_BALANCE_RATIO {
            // 3+ comparable peaks. Two shapes produce this: a genuine
            // N-column prose layout (newspaper / magazine / multi-col
            // reference list) or a data table. We can't bail on both —
            // bailing leaves N-column prose joined line-by-line, which the
            // borderless table detector then shreds into a fake table.
            // Keep every peak balanced against the largest and let the
            // gap logic cut the leftmost gutter; recursion re-runs on each
            // side and peels the remaining columns one at a time. The
            // downstream dominance + min-gap guards still reject tables
            // whose cells don't tightly left-cluster (numeric columns are
            // typically right-aligned / centered, so they never form
            // strong left-edge peaks and fail dominance here).
            let before = peaks.len();
            peaks.retain(|(_, c)| {
                largest <= 0.0 || (*c as f32) / largest >= XY_COLUMN_PEAK_BALANCE_RATIO
            });
            if peaks.len() < 2 {
                reject!("peaks>2 but <2 balanced (largest={largest})");
            }
            if dbg {
                eprintln!(
                    "[xy col-keep-N] kept {} of {} balanced peaks (N-column); xs=[{}]",
                    peaks.len(),
                    before,
                    peaks
                        .iter()
                        .map(|(x, c)| format!("{x:.0}:{c}"))
                        .collect::<Vec<_>>()
                        .join(",")
                );
            }
        } else {
            // Top-2 dominate → assume real columns + weak floats. Keep
            // top-2 (re-sorted left→right) and continue.
            let mut top2 = vec![by_count[0], by_count[1]];
            top2.sort_by(|a, b| a.0.total_cmp(&b.0));
            if dbg {
                eprintln!(
                    "[xy col-keep-top2] dropped {} weak peaks; kept xs=[{:.0}:{},{:.0}:{}]",
                    peaks.len() - 2,
                    top2[0].0,
                    top2[0].1,
                    top2[1].0,
                    top2[1].1
                );
            }
            peaks = top2;
        }
    }
    // Fill-density gate: prose columns fill each inter-peak slot nearly
    // edge-to-edge; table cells leave wide whitespace. Applies to ALL peak
    // counts including 2 — a 2-col label/value table (description list)
    // produces the same two left-edge peaks as 2-col prose, and slicing it
    // strands each half in its own leaf where no table detector can see
    // the rows whole. Per band, measure each column's covered span / slot
    // width; if too sparse, treat as a table and bail.
    {
        let mut n = peaks.len();
        let mut covered_sum = vec![0.0f32; n];
        let mut band_count = vec![0usize; n];
        let mut kk = 0;
        while kk < sorted.len() {
            let by = items[sorted[kk]].item.y;
            let mut jj = kk;
            while jj < sorted.len() && (items[sorted[jj]].item.y - by).abs() <= band_tol {
                jj += 1;
            }
            let mut lo_x = vec![f32::INFINITY; n];
            let mut hi_x = vec![f32::NEG_INFINITY; n];
            for &idx in &sorted[kk..jj] {
                let it = &items[idx].item;
                if !it.x.is_finite() {
                    continue;
                }
                // Peak centers are bucket-quantized, so a column's items can
                // start up to a bucket left of its peak center; without the
                // slack they all bin into the previous slot and that column
                // reads as empty (its sparse cells never get fill-checked).
                let mut c = 0;
                while c + 1 < n && it.x >= peaks[c + 1].0 - bucket_pt {
                    c += 1;
                }
                lo_x[c] = lo_x[c].min(it.x);
                hi_x[c] = hi_x[c].max(it.x + it.width.max(0.0));
            }
            for c in 0..n {
                if hi_x[c] > lo_x[c] {
                    let col_w = if c + 1 < n {
                        peaks[c + 1].0 - peaks[c].0
                    } else {
                        max_x - peaks[c].0
                    };
                    if col_w > 1.0 {
                        covered_sum[c] += ((hi_x[c] - lo_x[c]) / col_w).min(1.0);
                        band_count[c] += 1;
                    }
                }
            }
            kk = jj;
        }
        // Two-part gate that distinguishes prose columns from tables:
        //
        //   1. min_fill floor (MIN_FILL_HARD_FLOOR = 0.38) — rejects
        //      pages where any column has narrow text in many bands
        //      (e.g. table with numeric column). Real tables have
        //      at least one column with fill ≤ 0.36.
        //   2. Band-count-weighted avg_fill ≥ XY_COLUMN_MIN_FILL —
        //      catches table layouts where ALL columns are uniformly
        //      narrow (no column dominates the weighted average).
        //
        // Both must pass. The min-only gate was too brittle: a real
        // 3-column prose page with one page-bottom column having ~4
        // bands of short text dragged the min below 0.55 even though
        // every meaningful column was clearly prose. The avg-only
        // gate let through tables where the narrow column had enough
        // bands to be informative but its weight was diluted by
        // wider neighboring columns.
        let mut min_fill = f32::INFINITY;
        let mut weighted_sum = 0.0f32;
        let mut weighted_n = 0.0f32;
        let mut per_col_fill: Vec<f32> = Vec::with_capacity(n);
        for c in 0..n {
            if band_count[c] > 0 {
                let f = covered_sum[c] / band_count[c] as f32;
                per_col_fill.push(f);
                min_fill = min_fill.min(f);
                weighted_sum += f * band_count[c] as f32;
                weighted_n += band_count[c] as f32;
            }
        }
        let mut avg_fill = if weighted_n > 0.0 {
            weighted_sum / weighted_n
        } else {
            f32::INFINITY
        };
        if dbg {
            eprintln!(
                "[xy col-fill] avg_fill={avg_fill:.2} min_fill={min_fill:.2} (gate={XY_COLUMN_MIN_FILL}) per_col=[{}] band_counts=[{}]",
                per_col_fill
                    .iter()
                    .map(|f| format!("{f:.2}"))
                    .collect::<Vec<_>>()
                    .join(","),
                band_count
                    .iter()
                    .map(|c| c.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            );
        }
        const MIN_FILL_HARD_FLOOR: f32 = 0.38;
        // Figure-scatter phantom column: on a 2-column page with an embedded
        // plot (scatter cloud, axis tick labels), the plot's x-spread items
        // form a third peak that is BOTH sparsely-banded (far fewer text rows
        // than the real columns) and low-fill. Left in, it drags min_fill
        // under the floor and flips the whole region to `tabular`, suppressing
        // the genuine column split. A real narrow table column instead has a
        // band count comparable to its siblings (one cell per row), so the
        // band-count ratio cleanly separates the two. Drop phantom peaks when
        // ≥2 well-filled columns survive, then re-run the gate on them.
        if n > 2 && std::env::var("LITEPARSE_DISABLE_PHANTOM_COL_DROP").is_err() {
            let max_bands = band_count.iter().copied().max().unwrap_or(0);
            if max_bands > 0 {
                let band_floor =
                    (max_bands as f32 * XY_COLUMN_PHANTOM_BAND_FRACTION).ceil() as usize;
                let col_fill = |c: usize| -> f32 {
                    if band_count[c] > 0 {
                        covered_sum[c] / band_count[c] as f32
                    } else {
                        0.0
                    }
                };
                let survivors: Vec<usize> = (0..n)
                    .filter(|&c| !(band_count[c] < band_floor && col_fill(c) < MIN_FILL_HARD_FLOOR))
                    .collect();
                let survivors_filled = survivors
                    .iter()
                    .all(|&c| col_fill(c) >= MIN_FILL_HARD_FLOOR);
                if survivors.len() >= 2 && survivors.len() < n && survivors_filled {
                    let kept: Vec<(f32, usize)> = survivors.iter().map(|&c| peaks[c]).collect();
                    let mut new_min = f32::INFINITY;
                    let mut wsum = 0.0f32;
                    let mut wn = 0.0f32;
                    for &c in &survivors {
                        let f = col_fill(c);
                        new_min = new_min.min(f);
                        wsum += f * band_count[c] as f32;
                        wn += band_count[c] as f32;
                    }
                    if dbg {
                        eprintln!(
                            "[xy col-phantom] dropped {} sparse low-fill peak(s) (band_floor={band_floor}); kept xs=[{}]",
                            n - survivors.len(),
                            kept.iter()
                                .map(|(x, _)| format!("{x:.0}"))
                                .collect::<Vec<_>>()
                                .join(",")
                        );
                    }
                    peaks = kept;
                    n = peaks.len();
                    min_fill = new_min;
                    avg_fill = if wn > 0.0 { wsum / wn } else { f32::INFINITY };
                }
            }
        }
        // The min-floor catches ≥3-col tables whose narrow numeric column is
        // diluted out of the weighted average by wide neighbors. With only 2
        // columns no dilution is possible — a sparse column of two drags the
        // average itself — and a short second prose column (a page-bottom
        // column with a few bands) would false-positive here, suppressing a
        // needed column split AND the density V via the tabular flag.
        if n > 2 && min_fill.is_finite() && min_fill < MIN_FILL_HARD_FLOOR {
            *tabular = true;
            reject!("N-column min fill {min_fill:.2} < {MIN_FILL_HARD_FLOOR} (tabular column)");
        }
        if avg_fill.is_finite() && avg_fill < XY_COLUMN_MIN_FILL {
            *tabular = true;
            reject!(
                "N-column avg fill {avg_fill:.2} < {} (tabular)",
                XY_COLUMN_MIN_FILL
            );
        }
    }
    // Require the two peaks to together dominate the histogram — a real
    // 2-column layout has the great majority of items starting at one of
    // the two column-left edges. Tables, lists, and prose with scattered
    // indents distribute their item-starts across many x's and won't pass.
    let peak_sum: usize = peaks.iter().map(|(_, c)| *c).sum();
    let dominance = (peak_sum as f32) / (n_starts as f32);
    if dominance < XY_COLUMN_PEAK_DOMINANCE {
        reject!(
            "dominance {dominance:.2} < {} (peak_sum={peak_sum} line_starts={n_starts})",
            XY_COLUMN_PEAK_DOMINANCE
        );
    }
    // Require the peaks to be roughly balanced in size. A real multi-column
    // layout has comparable line counts in each column; a long-single-column
    // page that happens to have one bullet stack at a different x produces
    // a tall + tiny peak pair that should not trigger a column split. Check
    // the global min/max across all kept peaks so an N-column layout with one
    // short column is also rejected.
    let smallest = peaks.iter().map(|(_, c)| *c).min().unwrap() as f32;
    let largest = peaks.iter().map(|(_, c)| *c).max().unwrap() as f32;
    let balance = if largest > 0.0 {
        smallest / largest
    } else {
        0.0
    };
    if balance < XY_COLUMN_PEAK_BALANCE_RATIO {
        reject!(
            "balance {balance:.2} < {} (min={smallest} max={largest} npeaks={})",
            XY_COLUMN_PEAK_BALANCE_RATIO,
            peaks.len()
        );
    }

    // Cut at the leftmost gutter that clears `XY_COLUMN_MIN_GAP_FRACTION ×
    // total_w` (list-item / block-quote indents produce closely spaced peaks;
    // a real column gutter is much further apart). For a 2-column layout this
    // is the only gap. For N columns, peeling the leftmost column first and
    // letting the recursion re-run on the remainder splits the rest one
    // gutter at a time.
    let min_gap = total_w * XY_COLUMN_MIN_GAP_FRACTION;
    let mut best: Option<(f32, usize)> = None;
    for w_idx in 0..peaks.len() - 1 {
        let gap = peaks[w_idx + 1].0 - peaks[w_idx].0;
        if gap >= min_gap {
            best = Some((gap, w_idx));
            break;
        }
    }
    let li = match best {
        Some((_, li)) => li,
        None => reject!(
            "no gap >= {:.0} (min_gap_frac={} total_w={total_w:.0}); peak_xs=[{:.0},{:.0}] gap={:.0}",
            min_gap,
            XY_COLUMN_MIN_GAP_FRACTION,
            peaks[0].0,
            peaks[1].0,
            peaks[1].0 - peaks[0].0
        ),
    };
    // Place the cut at the actual gutter, not midway between peak centers.
    // Peaks mark column LEFT edges, so the center midpoint lands inside the
    // left column's text span and misroutes its mid-line items under
    // partition_by_item. The gutter spans from the rightmost left-side item
    // extent to the right column's leftmost counted start — NOT the peak
    // center: a right-aligned numeric column scatters its starts left of
    // the bucket-quantized center, and a cut placed at the center would
    // route those values into the left column.
    let mid_centers = (peaks[li].0 + peaks[li + 1].0) * 0.5;
    let right_min_start = counted_starts
        .iter()
        .copied()
        .filter(|&x| x > mid_centers)
        .fold(f32::INFINITY, f32::min);
    let boundary = if right_min_start.is_finite() {
        right_min_start
    } else {
        peaks[li + 1].0
    };
    let mut gutter_left = f32::NEG_INFINITY;
    for &i in idxs {
        let it = &items[i].item;
        if it.x.is_finite() {
            let r = it.x + it.width.max(0.0);
            if r <= boundary - 1.0 && r > gutter_left {
                gutter_left = r;
            }
        }
    }
    let cut_x = if gutter_left.is_finite() && gutter_left > peaks[li].0 {
        (gutter_left + boundary) * 0.5
    } else {
        mid_centers
    };
    // Validate cut lands inside the bbox; if bbox-clamping moved the bbox
    // off the items (some PDFs have content with negative x), fall back to
    // an item-relative midpoint check.
    if cut_x <= min_x + 1.0 || cut_x >= max_x - 1.0 {
        reject!("cut_x {cut_x:.0} outside extent [{min_x:.0},{max_x:.0}]");
    }
    // High finite score so this beats every density cut but stays below
    // banner's infinity.
    Some(CutCandidate {
        axis: CutAxis::Vertical,
        position: cut_x,
        score: 1.0e9,
    })
}

/// X position of the vertical gutter that splits `idxs` (within `bbox`) into
/// columns — from the density V-cut or the column-start histogram fallback.
/// `None` when the region is single-column. Used to decide whether peeling a
/// wide spanning line above this region is productive.
fn column_cut_x(
    items: &[ProjectedTextItem],
    idxs: &[usize],
    bbox: &Rect,
    median_h: f32,
    figures: &[Rect],
) -> Option<f32> {
    xy_find_best_cut(items, idxs, bbox, CutAxis::Vertical, median_h, figures)
        .or_else(|| {
            let mut tabular = false;
            xy_find_column_cut(items, idxs, bbox, median_h, &mut tabular)
        })
        .map(|c| c.position)
}

/// Union horizontal extent (min x, max x) of `idxs`.
fn x_extent(items: &[ProjectedTextItem], idxs: &[usize]) -> (f32, f32) {
    let mut lo = f32::INFINITY;
    let mut hi = f32::NEG_INFINITY;
    for &i in idxs {
        let it = &items[i].item;
        lo = lo.min(it.x);
        hi = hi.max(it.x + it.width.max(0.0));
    }
    (lo, hi)
}

fn xy_find_banner_cut(
    items: &[ProjectedTextItem],
    idxs: &[usize],
    bbox: &Rect,
    median_h: f32,
) -> Option<CutCandidate> {
    if bbox.width <= 1.0 || idxs.len() < 2 {
        return None;
    }
    let mut sorted: Vec<usize> = idxs.to_vec();
    sorted.sort_by(|&a, &b| items[a].item.y.total_cmp(&items[b].item.y));
    let band_tol = (median_h * 0.5).max(2.0);
    let min_clearance = (median_h * XY_BANNER_CLEARANCE_FACTOR).max(8.0);
    let width_threshold = XY_BANNER_WIDTH_FRACTION * bbox.width;

    // Group items into y-bands. A new item joins the current band when its top
    // is within `band_tol` of either the band's center or its existing bottom
    // (handles slight baseline variation).
    let mut bands: Vec<(f32, f32, f32, f32)> = Vec::new(); // (y_min, y_max, x_min, x_max)
    for &i in &sorted {
        let it = &items[i].item;
        let y0 = it.y;
        let y1 = it.y + it.height.max(0.0);
        let x0 = it.x;
        let x1 = it.x + it.width.max(0.0);
        let mut merged = false;
        if let Some(last) = bands.last_mut() {
            let center = (last.0 + last.1) * 0.5;
            if (y0 - center).abs() <= band_tol || y0 <= last.1 + band_tol {
                last.0 = last.0.min(y0);
                last.1 = last.1.max(y1);
                last.2 = last.2.min(x0);
                last.3 = last.3.max(x1);
                merged = true;
            }
        }
        if !merged {
            bands.push((y0, y1, x0, x1));
        }
    }
    if bands.is_empty() {
        return None;
    }
    let is_wide = |b: &(f32, f32, f32, f32)| -> bool { (b.3 - b.2) >= width_threshold };

    // Walk top→bottom. For each run of consecutive wide bands (close enough to
    // each other to count as a single banner stack), check above/below clearance
    // and emit a cut if isolated.
    let mut i = 0;
    while i < bands.len() {
        if !is_wide(&bands[i]) {
            i += 1;
            continue;
        }
        let start = i;
        let mut end = i;
        // Extend through consecutive wide bands separated by less than the
        // clearance threshold — multi-line title + authors counts as one
        // banner stack.
        while end + 1 < bands.len()
            && is_wide(&bands[end + 1])
            && bands[end + 1].0 - bands[end].1 < min_clearance
        {
            end += 1;
        }
        let banner_top = bands[start].0;
        let banner_bot = bands[end].1;
        let above_y = if start > 0 {
            Some(bands[start - 1].1)
        } else {
            None
        };
        let below_y = if end + 1 < bands.len() {
            Some(bands[end + 1].0)
        } else {
            None
        };
        let above_clear = above_y.is_none_or(|ay| banner_top - ay >= min_clearance);
        let below_clear = below_y.is_none_or(|by| by - banner_bot >= min_clearance);

        if above_clear && below_clear {
            // Prefer cutting BELOW the banner so it becomes the top region of
            // the split (matches natural reading order). Fall back to cutting
            // above only when there's nothing below to cut against.
            let cut_y = if let Some(by) = below_y {
                (banner_bot + by) * 0.5
            } else if let Some(ay) = above_y {
                (ay + banner_top) * 0.5
            } else {
                // Banner spans the whole region — no productive cut.
                i = end + 1;
                continue;
            };
            // Skip degenerate cuts that would leave one side empty.
            if cut_y > bbox.y + 1.0 && cut_y < bbox.y + bbox.height - 1.0 {
                return Some(CutCandidate {
                    axis: CutAxis::Horizontal,
                    position: cut_y,
                    score: f32::INFINITY,
                });
            }
        }
        i = end + 1;
    }
    None
}

fn xy_cut_rec(
    items: &[ProjectedTextItem],
    idxs: Vec<usize>,
    bbox: Rect,
    depth: u32,
    median_h: f32,
    figures: &[Rect],
) -> Region {
    if idxs.len() <= XY_MIN_ITEMS_TO_CUT || depth >= XY_MAX_DEPTH {
        return Region {
            bbox,
            kind: RegionKind::Leaf { item_indices: idxs },
        };
    }
    // Banner cut wins over density-based valleys. Score is f32::INFINITY so
    // the existing comparison logic naturally selects it.
    let banner = xy_find_banner_cut(items, &idxs, &bbox, median_h);
    let h = xy_find_best_cut(items, &idxs, &bbox, CutAxis::Horizontal, median_h, figures);
    let v = xy_find_best_cut(items, &idxs, &bbox, CutAxis::Vertical, median_h, figures);
    // Fallback column-start histogram. Only consulted when BOTH density
    // cuts returned None — those are the cases (wide-heading filled gutter,
    // figure-straddle layouts) where the column structure is real but the
    // density projection can't see it. If either density cut found a
    // valley, trust it instead of firing the histogram fallback.
    // Run the histogram probe unconditionally: besides being a fallback cut
    // source, its fill gate is the region-level table detector — when it
    // rejects with "tabular", density V-cuts must not slice the table's
    // inter-column gutters either (the dominant column-major-table failure).
    let mut region_tabular = false;
    let column_full = xy_find_column_cut(items, &idxs, &bbox, median_h, &mut region_tabular);
    let column = if v.is_none() && h.is_none() {
        column_full
    } else {
        None
    };
    let debug_xy = std::env::var("LITEPARSE_DEBUG_XY").is_ok();
    let disable_vcut_minlines = std::env::var("LITEPARSE_DISABLE_VCUT_MINLINES").is_ok();
    if debug_xy && depth == 0 && std::env::var("LITEPARSE_DEBUG_ITEMS").is_ok() {
        for &i in &idxs {
            let it = &items[i].item;
            eprintln!(
                "[xy item] x={:.1} w={:.1} y={:.1} h={:.1} text={:?}",
                it.x,
                it.width,
                it.y,
                it.height,
                &it.text.chars().take(80).collect::<String>()
            );
        }
    }
    if debug_xy {
        // Also probe what the column histogram WOULD return if we ran it
        // unconditionally — this is the diagnostic that tells us whether
        // reordering V/H/column priority would unlock more column splits.
        let column_probe = column_full;
        let pad = "  ".repeat(depth as usize);
        let fmt = |c: &Option<CutCandidate>| {
            c.as_ref()
                .map(|c| format!("{:?}@{:.1}/{:.3}", c.axis, c.position, c.score))
                .unwrap_or_else(|| "-".to_string())
        };
        let fmt_pos = |p: &Option<f32>| {
            p.as_ref()
                .map(|p| format!("{p:.1}"))
                .unwrap_or_else(|| "-".to_string())
        };
        let (ex0, ex1) = x_extent(items, &idxs);
        let lines = xy_distinct_lines(items, &idxs, median_h);
        eprintln!(
            "[xy d={depth}]{pad} bbox=({:.0},{:.0} {:.0}x{:.0}) items={} lines={} x_ext=[{ex0:.1},{ex1:.1}] banner={} h={} v={} col={} col_probe={}",
            bbox.x,
            bbox.y,
            bbox.width,
            bbox.height,
            idxs.len(),
            lines,
            fmt(&banner),
            fmt(&h),
            fmt(&v),
            fmt_pos(&column.as_ref().map(|c| c.position)),
            fmt_pos(&column_probe.as_ref().map(|c| c.position)),
        );
    }
    // Build a priority-ordered candidate list.
    //
    // **Primary** is the cut chosen by the main resolution order
    // (banner → column → H/V resolved by score margin). **Fallback** is
    // the column-histogram probe — but ONLY that, not density V or the
    // other density axis. The column histogram has strong discriminators
    // (2 strong dominant balanced peaks); a density V valley that lost to
    // H by the score margin is often a layout coincidence (mid-line word
    // clustering) we shouldn't trust as a fallback. Falling through to
    // the histogram-only fallback — rather than collapsing to a leaf —
    // is what unblocks 2-column pages whose density H-cut peels a
    // footer/header but trips the min-lines guard.
    let (h_score, v_score) = (h.as_ref().map(|c| c.score), v.as_ref().map(|c| c.score));
    let prefer_h_over_v = match (h_score, v_score) {
        (Some(hs), Some(vs)) => vs <= hs * XY_V_PREFERENCE_MARGIN,
        _ => true,
    };
    let mut candidates: Vec<CutCandidate> = Vec::new();
    if let Some(bc) = banner {
        candidates.push(bc);
    }
    if let Some(cc) = column {
        candidates.push(cc);
    }
    // Primary density cut, then the other axis as fallback. Without the
    // alternate-axis fallback, a low-impact H-cut that wins by the score
    // margin (e.g. peeling a 1-line footer) trips the min-lines guard and
    // collapses the whole region to a single leaf — masking the real
    // column structure the V-cut would have revealed.
    // When the histogram fill gate classified this region as tabular, a
    // density V valley here is a table gutter, not a column boundary —
    // suppress it so the table reaches the table detectors row-major.
    let v = if region_tabular {
        if debug_xy && v.is_some() {
            let pad = "  ".repeat(depth as usize);
            eprintln!("[xy d={depth}]{pad} -> SUPPRESS density V (region tabular)");
        }
        None
    } else {
        v
    };
    if prefer_h_over_v {
        if let Some(hc) = h {
            candidates.push(hc);
        }
        if let Some(vc) = v {
            candidates.push(vc);
        }
    } else {
        if let Some(vc) = v {
            candidates.push(vc);
        }
        if let Some(hc) = h {
            candidates.push(hc);
        }
    }
    // Final fallback: re-use the unconditional column-histogram probe.
    // Skipped when an existing candidate is already a histogram-derived
    // column cut (score = 1.0e9 from `xy_find_column_cut`).
    if !candidates
        .iter()
        .any(|c| c.axis == CutAxis::Vertical && (c.score - 1.0e9).abs() < 1.0)
        && let Some(cc) = column_full
    {
        candidates.push(cc);
    }

    for cut in candidates {
        if debug_xy {
            let pad = "  ".repeat(depth as usize);
            eprintln!(
                "[xy d={depth}]{pad} -> TRY {:?}@{:.1}/{:.3}",
                cut.axis, cut.position, cut.score
            );
        }
        let mut left: Vec<usize> = Vec::new();
        let mut right: Vec<usize> = Vec::new();
        // Column-histogram V-cuts (score = 1.0e9) split between two column
        // left-edge clusters. The cut sits at the midpoint between the two
        // peak x's, which is typically far inside the left column. PDFium
        // routinely emits a single visual line as multiple items (line-start
        // + mid-line emphasis fragments + late-line clusters); each item has
        // its own x, so an item-by-item partition by `it.x` will split one
        // logical line across both sides whenever a continuation fragment
        // starts past `cut.position`. Verified on `031a888e…page_1` where
        // a histogram V-cut at x=360.8 inside column 2 routed the body-start
        // fragments at x=312 LEFT but their same-baseline continuation
        // "In Section 2 the" at x=474 RIGHT, orphaning the snippet from its
        // paragraph and producing a stranded block downstream.
        //
        // Fix: for histogram V-cuts, first measure how many y-bands have items
        // on *both* sides of the cut ("straddling rows"). A real column
        // boundary has every body row straddling (text in left + right column
        // on the same baseline) — partition by `it.x` so each column gets its
        // items. An intra-column histogram split (031a888e case: a single line
        // wrapped into multiple items at different x's) has almost no
        // straddling rows — partition by row leftmost-x so a wrapped line
        // stays whole. The threshold (>~25% of rows straddle) cleanly
        // separates: 0145bc47 (every body row straddles, ~100%) gets per-item
        // partition; 031a888e (1 of ~45 rows straddles, ~2%) gets per-row.
        //
        // Density V-cuts (white-space valleys) still use centroid: they only
        // fire where a real text-free vertical band exists, so an item that
        // crosses a density cut is genuinely wider than the column.
        let is_histogram_v_cut = cut.axis == CutAxis::Vertical && (cut.score - 1.0e9).abs() < 1.0;
        if is_histogram_v_cut {
            // Sort by y then x so items on one baseline cluster together.
            let mut sorted_idxs: Vec<usize> = idxs.clone();
            sorted_idxs.sort_by(|&a, &b| {
                items[a]
                    .item
                    .y
                    .total_cmp(&items[b].item.y)
                    .then(items[a].item.x.total_cmp(&items[b].item.x))
            });
            let band_tol = (median_h * 0.5).max(2.0);

            // First pass: count rows with items entirely-left, entirely-right,
            // or straddling the cut. Decides the partition strategy.
            let (mut rows_total, mut rows_straddle) = (0usize, 0usize);
            let mut k = 0;
            while k < sorted_idxs.len() {
                let band_y = items[sorted_idxs[k]].item.y;
                let mut j = k;
                let mut has_left = false;
                let mut has_right = false;
                while j < sorted_idxs.len()
                    && (items[sorted_idxs[j]].item.y - band_y).abs() <= band_tol
                {
                    let ix = items[sorted_idxs[j]].item.x;
                    if ix.is_finite() {
                        if ix < cut.position {
                            has_left = true;
                        } else {
                            has_right = true;
                        }
                    }
                    j += 1;
                }
                if has_left || has_right {
                    rows_total += 1;
                    if has_left && has_right {
                        rows_straddle += 1;
                    }
                }
                k = j;
            }
            // 25% threshold: when a quarter or more of rows have content on
            // both sides of the cut, it's a real column boundary (every body
            // row has text in both columns). Below that, the cut sits inside
            // a single column and the rare straddler is an intra-column
            // wrapped line that must not be torn apart.
            const STRADDLE_PARTITION_FRACTION: f32 = 0.25;
            let partition_by_item = rows_total > 0
                && (rows_straddle as f32) / (rows_total as f32) >= STRADDLE_PARTITION_FRACTION;
            if debug_xy {
                let pad = "  ".repeat(depth as usize);
                eprintln!(
                    "[xy d={depth}]{pad}    hist-v straddle={}/{} → partition_by_{}",
                    rows_straddle,
                    rows_total,
                    if partition_by_item { "item" } else { "row" }
                );
            }

            if partition_by_item {
                for &i in &idxs {
                    if items[i].item.x < cut.position {
                        left.push(i);
                    } else {
                        right.push(i);
                    }
                }
            } else {
                let mut k = 0;
                while k < sorted_idxs.len() {
                    let band_y = items[sorted_idxs[k]].item.y;
                    let mut j = k;
                    let mut min_x = f32::INFINITY;
                    while j < sorted_idxs.len()
                        && (items[sorted_idxs[j]].item.y - band_y).abs() <= band_tol
                    {
                        let ix = items[sorted_idxs[j]].item.x;
                        if ix.is_finite() && ix < min_x {
                            min_x = ix;
                        }
                        j += 1;
                    }
                    let side_left = min_x < cut.position;
                    for &i in &sorted_idxs[k..j] {
                        if side_left {
                            left.push(i);
                        } else {
                            right.push(i);
                        }
                    }
                    k = j;
                }
            }
        } else {
            for &i in &idxs {
                let it = &items[i].item;
                let split_at = match cut.axis {
                    CutAxis::Horizontal => it.y + it.height.max(0.0) * 0.5,
                    CutAxis::Vertical => it.x + it.width.max(0.0) * 0.5,
                };
                if split_at < cut.position {
                    left.push(i);
                } else {
                    right.push(i);
                }
            }
        }
        if debug_xy {
            let pad = "  ".repeat(depth as usize);
            eprintln!(
                "[xy d={depth}]{pad}    partition: left={} right={}",
                left.len(),
                right.len()
            );
        }
        if left.is_empty() || right.is_empty() {
            if debug_xy {
                let pad = "  ".repeat(depth as usize);
                eprintln!("[xy d={depth}]{pad}    -> degenerate, try next");
            }
            continue;
        }
        let (left_bbox, right_bbox) = xy_split_bbox(&bbox, &cut);

        // Banner cuts (score = ∞) at the page root are explicitly meant to
        // isolate single-line wide headers like titles, so skip the
        // min-lines guard for them. At deeper recursion levels we keep the
        // guard — an aggressive single-line banner cut mid-recursion
        // fragments paragraphs around emphasized lines (e.g. a centered
        // "Figure N caption" sandwiched between body paragraphs would
        // otherwise carve out its own sliver region and break paragraph
        // continuation).
        let allow_single_line =
            cut.axis == CutAxis::Horizontal && cut.score.is_infinite() && depth == 0;
        if cut.axis == CutAxis::Horizontal && !allow_single_line {
            let lc = xy_distinct_lines(items, &left, median_h);
            let rc = xy_distinct_lines(items, &right, median_h);
            // Rescue: a single wide spanning line (e.g. a centered
            // authors / affiliation line above a two-column body) is
            // worth isolating even mid-recursion when peeling it reveals
            // the column gutter underneath. Otherwise the spanning line
            // straddles the gutter, the V-cut can never fire, and the
            // whole body interleaves column-by-column. The density H-cut
            // that peels it has a finite score (the line is often just
            // under the banner width threshold), so this is NOT gated on
            // a banner cut. Discriminator vs. carving a single-column
            // caption out of running prose: the thin side must be one
            // line whose x-extent crosses the gutter revealed on the
            // other (multi-line) side.
            let spanning_rescue = {
                let thin_wide = if lc < XY_MIN_LINES_PER_H_SIDE && rc >= XY_MIN_LINES_PER_H_SIDE {
                    Some(((&left, lc), (&right, &right_bbox)))
                } else if rc < XY_MIN_LINES_PER_H_SIDE && lc >= XY_MIN_LINES_PER_H_SIDE {
                    Some(((&right, rc), (&left, &left_bbox)))
                } else {
                    None
                };
                thin_wide.is_some_and(|((thin, thin_lines), (wide, wide_bbox))| {
                    thin_lines <= 1
                        && column_cut_x(items, wide, wide_bbox, median_h, figures).is_some_and(
                            |gx| {
                                let (tx0, tx1) = x_extent(items, thin);
                                tx0 < gx - 1.0 && tx1 > gx + 1.0
                            },
                        )
                })
            };
            if (lc < XY_MIN_LINES_PER_H_SIDE || rc < XY_MIN_LINES_PER_H_SIDE) && !spanning_rescue {
                if debug_xy {
                    let pad = "  ".repeat(depth as usize);
                    eprintln!(
                        "[xy d={depth}]{pad}    -> min-lines fail (lc={lc} rc={rc}), try next"
                    );
                }
                continue;
            }
        }
        // Vertical (column) min-lines guard. A real column gutter is sustained
        // across multiple stacked lines on both sides. A vertical cut that
        // isolates a single-line "column" is a false gutter: in single-column
        // justified prose, a short final line plus a wide inter-word gap in the
        // full-width line above it align an empty vertical band that is not a
        // column boundary. Without this, that band splits the physical line
        // into two sibling regions and the markdown emitter renders them out of
        // order (e.g. "…and its share repurchases are 100%" → the tail is
        // sliced off into its own region). The initial true 2-column split has
        // many lines per side, so this never blocks real columns.
        if cut.axis == CutAxis::Vertical && !disable_vcut_minlines {
            let lc = xy_distinct_lines(items, &left, median_h);
            let rc = xy_distinct_lines(items, &right, median_h);
            if lc < XY_MIN_LINES_PER_H_SIDE || rc < XY_MIN_LINES_PER_H_SIDE {
                if debug_xy {
                    let pad = "  ".repeat(depth as usize);
                    eprintln!(
                        "[xy d={depth}]{pad}    -> v-cut min-lines fail (lc={lc} rc={rc}), try next"
                    );
                }
                continue;
            }
        }

        if debug_xy {
            let pad = "  ".repeat(depth as usize);
            eprintln!(
                "[xy d={depth}]{pad} -> COMMIT {:?}@{:.1}",
                cut.axis, cut.position
            );
        }
        let left_region = xy_cut_rec(items, left, left_bbox, depth + 1, median_h, figures);
        let right_region = xy_cut_rec(items, right, right_bbox, depth + 1, median_h, figures);
        return Region {
            bbox,
            kind: RegionKind::Split {
                axis: cut.axis,
                children: vec![left_region, right_region],
            },
        };
    }

    if debug_xy {
        let pad = "  ".repeat(depth as usize);
        eprintln!("[xy d={depth}]{pad} -> LEAF (no valid cut)");
    }
    Region {
        bbox,
        kind: RegionKind::Leaf { item_indices: idxs },
    }
}

/// Build the page's XY-cut region tree.
///
/// `figures` is an optional set of figure-region bounding rectangles (from
/// `figure_cluster::detect_figure_rects`); pass `&[]` to disable obstacle
/// seeding. Figures cause the recursion to favor cuts around them.
pub(crate) fn xy_cut(
    items: &[ProjectedTextItem],
    page_width: f32,
    page_height: f32,
    figures: &[Rect],
) -> Region {
    let all: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, it)| !it.item.text.is_empty())
        .map(|(i, _)| i)
        .collect();
    if all.is_empty() {
        return Region {
            bbox: Rect {
                x: 0.0,
                y: 0.0,
                width: page_width.max(1.0),
                height: page_height.max(1.0),
            },
            kind: RegionKind::Leaf {
                item_indices: Vec::new(),
            },
        };
    }
    let bbox = xy_root_bbox(items, page_width, page_height);
    let median_h = xy_median_line_height(items, &all);
    xy_cut_rec(items, all, bbox, 0, median_h, figures)
}

/// Pre-order walk of leaves. Each leaf yields its `region_path` (index trail
/// from the root) and item indices. Leaves with no items are skipped.
fn xy_walk_leaves(region: &Region, path: &mut Vec<u16>, out: &mut Vec<(Vec<u16>, Vec<usize>)>) {
    match &region.kind {
        RegionKind::Leaf { item_indices } => {
            if !item_indices.is_empty() {
                out.push((path.clone(), item_indices.clone()));
            }
        }
        RegionKind::Split { children, .. } => {
            for (i, child) in children.iter().enumerate() {
                path.push(i as u16);
                xy_walk_leaves(child, path, out);
                path.pop();
            }
        }
    }
}

/// Group `items` into per-leaf lines using the XY-cut region tree, then derive
/// per-line aggregates. Leaves are walked pre-order so reading order follows
/// the layout — top→bottom bands, left→right columns, recursively. Returns
/// the populated lines and the region tree so callers can persist it on
/// `ParsedPage`.
pub(crate) fn build_projected_lines(
    items: &[ProjectedTextItem],
    page_width: f32,
    page_height: f32,
    figures: &[Rect],
) -> (Vec<ProjectedLine>, Region) {
    if items.is_empty() {
        return (Vec::new(), Region::default());
    }

    let region = xy_cut(items, page_width, page_height, figures);
    let mut leaves: Vec<(Vec<u16>, Vec<usize>)> = Vec::new();
    xy_walk_leaves(&region, &mut Vec::new(), &mut leaves);

    // Figures used for the per-line `in_figure` heading-exclusion flag. Drop
    // page-dominating figures: on posters / slides / infographics the whole
    // page is one large graphic and its big text *is* the heading content, so
    // excluding it would demote real headings. A chart on an academic page
    // covers only a fraction of the page — that's the pollution we want out of
    // the heading histogram.
    let page_area = (page_width.max(1.0)) * (page_height.max(1.0));
    let heading_excl_figures: Vec<Rect> = figures
        .iter()
        .filter(|f| f.width * f.height < page_area * 0.55)
        .cloned()
        .collect();

    let mut out: Vec<ProjectedLine> = Vec::new();
    for (path, indices) in leaves {
        // Sort within the leaf by y, tie-break by x. `build_one_line` re-sorts
        // by x for left→right concatenation; the y-banding loop here only
        // needs y order.
        let mut sorted = indices;
        sorted.sort_by(|&a, &b| {
            items[a]
                .item
                .y
                .total_cmp(&items[b].item.y)
                .then(items[a].item.x.total_cmp(&items[b].item.x))
        });

        let mut current: Vec<usize> = Vec::new();
        let mut current_y: f32 = 0.0;
        let mut current_h: f32 = 0.0;
        // PDFium occasionally reports anomalously large item heights (e.g.
        // 56pt for a single-word run whose real glyph height is ~13pt) when
        // the font's bounding box / line-height is baked into the text-matrix
        // scale. Without a cap, the y-band tolerance `max(h) * 0.5` swallows
        // multiple distinct baselines into one projected line (visible on
        // docs 121, 122 — fill-in-the-blank lab worksheets). Clamp at 24pt
        // for the banding decision; the actual stored line bbox is unchanged.
        const Y_BAND_HEIGHT_CAP: f32 = 24.0;
        for idx in sorted {
            let y = items[idx].item.y;
            let raw_h = items[idx].item.height;
            let h = raw_h.clamp(1.0, Y_BAND_HEIGHT_CAP);
            if current.is_empty() {
                current.push(idx);
                current_y = y;
                current_h = h;
                continue;
            }
            // Use the SMALLER of the two heights for the y-band tolerance —
            // matching baselines (same row) differ by ≪ either glyph height,
            // while the next-row-down baseline differs by ≈ the smaller
            // glyph's full height. Using `.max()` here is fooled by
            // matrix-inflated em-boxes: a 16pt heading row "swallows" a
            // 50pt-em-box body item on the row below because the tolerance
            // becomes 25pt. Reproducer: doc 122 ("III. Electrophorese" at
            // y=286.2 vs "Reagents:" at y=295.9, diff=9.7pt).
            // When the incoming item's raw bbox is wildly taller than the
            // current line's anchor (>2× and >Y_BAND_HEIGHT_CAP), this is
            // an em-box-inflated body item trying to crash into a heading
            // row. Halve the tolerance further so a small y-offset (4–7pt)
            // doesn't fuse them. Reproducer: doc 122 "Load the Gel"
            // (h=14.9, y=435.6) vs next-row "Use" (h=55.6, y=442.0,
            // diff=6.4pt vs 14.9*0.5=7.45 tolerance — too permissive).
            let height_mismatch = raw_h > Y_BAND_HEIGHT_CAP && raw_h > current_h * 2.0;
            let tol_factor = if height_mismatch { 0.3 } else { 0.5 };
            let same = (y - current_y).abs() < current_h.min(h) * tol_factor;
            if same {
                current.push(idx);
                current_y = current_y.min(y);
                current_h = current_h.max(h);
            } else {
                out.push(build_one_line(
                    items,
                    &current,
                    path.clone(),
                    &heading_excl_figures,
                ));
                current = vec![idx];
                current_y = y;
                current_h = h;
            }
        }
        if !current.is_empty() {
            out.push(build_one_line(
                items,
                &current,
                path.clone(),
                &heading_excl_figures,
            ));
        }
    }

    // Normalize `indent_x` to be leaf-relative: subtract each leaf's minimum
    // line bbox.x from every line in that leaf. This way list-nesting and
    // paragraph-indent comparisons in `markdown_layout.rs` use offsets from
    // the column's left edge rather than absolute page x. Multi-column pages
    // would otherwise see every column-2 line as "indented" relative to
    // column-1 lines.
    {
        use std::collections::HashMap;
        let mut leaf_min: HashMap<Vec<u16>, f32> = HashMap::new();
        for line in &out {
            let e = leaf_min
                .entry(line.region_path.clone())
                .or_insert(f32::INFINITY);
            if line.indent_x < *e {
                *e = line.indent_x;
            }
        }
        for line in &mut out {
            if let Some(min) = leaf_min.get(&line.region_path)
                && min.is_finite()
            {
                line.indent_x -= *min;
                if line.indent_x < 0.0 {
                    line.indent_x = 0.0;
                }
            }
        }
    }

    (out, region)
}

fn build_one_line(
    items: &[ProjectedTextItem],
    idxs: &[usize],
    region_path: Vec<u16>,
    figures: &[Rect],
) -> ProjectedLine {
    // Sort by x so concatenation reads left→right even if reading order had
    // rotated insertions.
    let mut sorted: Vec<usize> = idxs.to_vec();
    sorted.sort_by(|a, b| items[*a].item.x.total_cmp(&items[*b].item.x));

    let mut text = String::new();
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    // Original-coordinate bbox (pre-rotation-displacement). `figures` are in
    // original page space, so figure-overlap must be tested here, not against
    // the projected `bbox` below (which can be displaced onto a virtual canvas
    // by `handle_rotation_reading_order`).
    let mut omin_x = f32::INFINITY;
    let mut omin_y = f32::INFINITY;
    let mut omax_x = f32::NEG_INFINITY;
    let mut omax_y = f32::NEG_INFINITY;

    let mut size_weights: HashMap<u32, (f32, usize)> = HashMap::new();
    let mut height_weights: HashMap<u32, (f32, usize)> = HashMap::new();
    // Matrix-derived true on-page size (`Tf_size × text_matrix_scale`, computed
    // in extract.rs). For matrix-baked-size fonts this is the precise size the
    // raw `font_size` (≈1.0) hides; jitter-free, unlike bbox height.
    let mut font_height_weights: HashMap<u32, (f32, usize)> = HashMap::new();
    let mut name_weights: HashMap<String, usize> = HashMap::new();
    let mut bold_chars: usize = 0;
    let mut italic_chars: usize = 0;
    let mut mono_chars: usize = 0;
    let mut total_chars: usize = 0;
    let mut anchor_weights: HashMap<u8, usize> = HashMap::new();
    let mut mcid: Option<i32> = None;
    let mut spans: Vec<TextItem> = Vec::with_capacity(sorted.len());

    for (pos, &i) in sorted.iter().enumerate() {
        let proj = &items[i];
        let it = &proj.item;
        // `handle_rotation_reading_order` zeroes `item.rotation` after it
        // reorders cardinal-rotated text into reading order, so the span would
        // otherwise look horizontal to `is_rotated_line`. Restore the original
        // rotation onto the span clone (only consumed by markdown heading/TOC
        // rotation guards) so a sideways margin stamp doesn't pollute the
        // heading-size map.
        let mut span = it.clone();
        span.rotation = proj.orig_rotation;
        spans.push(span);

        // Concatenate item text. Use existing num_spaces from projection only as
        // a hint — the markdown emitter re-collapses whitespace, so we just
        // ensure there's *some* separation between adjacent items.
        if pos > 0 && !text.ends_with(' ') {
            text.push(' ');
        }
        text.push_str(&it.text);

        min_x = min_x.min(it.x);
        min_y = min_y.min(it.y);
        max_x = max_x.max(it.x + it.width);
        max_y = max_y.max(it.y + it.height);

        omin_x = omin_x.min(proj.orig_x);
        omin_y = omin_y.min(proj.orig_y);
        omax_x = omax_x.max(proj.orig_x + proj.orig_width);
        omax_y = omax_y.max(proj.orig_y + proj.orig_height);

        let n = it.text.chars().count().max(1);
        total_chars += n;

        if let Some(size) = it.font_size
            && size > 0.0
        {
            let key = (size * 100.0).round() as u32;
            let e = size_weights.entry(key).or_insert((size, 0));
            e.1 += n;
        }
        let h_key = (it.height.max(0.0) * 100.0).round() as u32;
        let e = height_weights.entry(h_key).or_insert((it.height, 0));
        e.1 += n;
        if let Some(fh) = it.font_height
            && fh > 1.5
        {
            let fh_key = (fh * 100.0).round() as u32;
            let e = font_height_weights.entry(fh_key).or_insert((fh, 0));
            e.1 += n;
        }

        if let Some(name) = &it.font_name {
            *name_weights.entry(name.clone()).or_insert(0) += n;
        }

        if is_bold_item(it) {
            bold_chars += n;
        }
        if is_italic_item(it) {
            italic_chars += n;
        }
        if is_mono_item(it) {
            mono_chars += n;
        }

        let akey = match proj.anchor {
            Anchor::Left => 0u8,
            Anchor::Right => 1,
            Anchor::Center => 2,
            Anchor::Floating => 3,
        };
        *anchor_weights.entry(akey).or_insert(0) += n;

        if mcid.is_none() {
            mcid = it.mcid;
        }
    }

    // NOTE on tie-breaks: `max_by_key` over a HashMap returns the *last* max it
    // iterates, and HashMap iteration order is randomized per process. Ties on
    // char-weight would therefore pick a different winner every run, making the
    // line's font size / name / anchor non-deterministic (and with them, heading
    // and emphasis classification downstream). We break every tie on the bucket
    // key so the result is stable across runs.
    let dominant_size_from_font = size_weights
        .iter()
        .max_by_key(|(k, (_, n))| (*n, **k))
        .map(|(_, (s, _))| *s)
        .unwrap_or(0.0);
    // Fallback: when PDFium reports font_size ≤ 1.5 (size baked into the text
    // matrix), use char-weighted bbox height so the size-dependent grouping
    // (tables, paragraphs) keeps its well-tuned behavior.
    let (dominant_font_size, font_size_is_estimated) = if dominant_size_from_font > 1.5 {
        (dominant_size_from_font, false)
    } else {
        let h = height_weights
            .iter()
            .max_by_key(|(k, (_, n))| (*n, **k))
            .map(|(_, (h, _))| *h)
            .unwrap_or(0.0);
        (h, true)
    };

    // Precise size for *heading detection only*: when the raw size is baked,
    // the matrix-derived `font_height` (Tf_size × text_matrix_scale) is the
    // jitter-free on-page size, far better than the bbox-height estimate above
    // for separating headings from body. Routed only to body-size + heading-map
    // (see `heading_font_size` doc on ProjectedLine); table/paragraph grouping
    // deliberately keeps the bbox-height value to avoid regressing table
    // structure.
    let heading_font_size = if dominant_size_from_font > 1.5 {
        None
    } else {
        font_height_weights
            .iter()
            .max_by_key(|(k, (_, n))| (*n, **k))
            .map(|(_, (h, _))| *h)
    };

    let dominant_font_name = name_weights
        .iter()
        .max_by_key(|(name, n)| (**n, *name))
        .map(|(name, _)| name.clone());

    let majority = |count: usize| total_chars > 0 && count * 2 > total_chars;

    let dominant_anchor_key = anchor_weights
        .iter()
        .max_by_key(|(k, n)| (**n, **k))
        .map(|(k, _)| *k)
        .unwrap_or(0);
    let anchor = match dominant_anchor_key {
        1 => Anchor::Right,
        2 => Anchor::Center,
        3 => Anchor::Floating,
        _ => Anchor::Left,
    };

    let bbox = Rect {
        x: if min_x.is_finite() { min_x } else { 0.0 },
        y: if min_y.is_finite() { min_y } else { 0.0 },
        width: if max_x.is_finite() && min_x.is_finite() {
            (max_x - min_x).max(0.0)
        } else {
            0.0
        },
        height: if max_y.is_finite() && min_y.is_finite() {
            (max_y - min_y).max(0.0)
        } else {
            0.0
        },
    };

    // Figure membership: ≥50% of the line's original-coordinate area inside a
    // detected figure that is also clearly taller than the line. The height
    // guard is what separates real chart pollution (axis labels, legends — set
    // inside tall plot regions) from a section heading sitting on a thin
    // colored background bar, which figure detection can mis-cluster as a
    // figure ~1 line tall. Only the former should lose heading candidacy.
    let line_h = (omax_y - omin_y).max(0.0);
    let in_figure = if omax_x > omin_x && line_h > 0.0 {
        let line_area = (omax_x - omin_x) * line_h;
        figures.iter().any(|f| {
            if f.height < line_h * 3.0 {
                return false;
            }
            let ix = (omax_x.min(f.x + f.width) - omin_x.max(f.x)).max(0.0);
            let iy = (omax_y.min(f.y + f.height) - omin_y.max(f.y)).max(0.0);
            line_area > 0.0 && (ix * iy) / line_area >= 0.5
        })
    } else {
        false
    };

    ProjectedLine {
        text,
        bbox: bbox.clone(),
        anchor,
        // Real column detection is deferred (carry-forward in MARKDOWN_PROGRESS).
        // `indent_x` mirrors bbox.x for now; the markdown emitter relies on
        // this for paragraph/list-indent comparisons within a single column.
        indent_x: bbox.x,
        dominant_font_size,
        font_size_is_estimated,
        heading_font_size,
        dominant_font_name,
        all_bold: majority(bold_chars),
        all_italic: majority(italic_chars),
        all_mono: majority(mono_chars),
        all_strike: false,
        spans,
        region_path,
        mcid,
        in_figure,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn projected_item(text: &str, y: f32, width: f32, height: f32) -> ProjectedTextItem {
        ProjectedTextItem {
            item: TextItem {
                text: text.to_string(),
                x: 10.0,
                y,
                width,
                height,
                ..Default::default()
            },
            snap: Snap::Left,
            anchor: Anchor::Left,
            is_dup: false,
            rendered: false,
            num_spaces: 0,
            force_unsnapped: false,
            is_margin_line_number: false,
            rotated: false,
            d: 0.0,
            orig_x: 10.0,
            orig_y: y,
            orig_width: width,
            orig_height: height,
            orig_rotation: 0.0,
        }
    }

    #[test]
    fn project_to_grid_handles_text_sparse_zero_width_items() {
        let page = Page {
            page_number: 1,
            page_width: 612.0,
            page_height: 792.0,
            graphics: Vec::new(),
            vector_graphics: None,
            text_items: Vec::new(),
            struct_nodes: Vec::new(),
            image_refs: Vec::new(),
        };
        let projection_boxes = vec![
            projected_item("", 10.0, 0.0, 10.0),
            projected_item("", 30.0, 0.0, 20.0),
        ];

        let (_, text) = project_to_grid(&page, projection_boxes);

        assert!(text.is_empty());
    }

    fn item_at(text: &str, x: f32, y: f32, w: f32, h: f32) -> ProjectedTextItem {
        ProjectedTextItem {
            item: TextItem {
                text: text.to_string(),
                x,
                y,
                width: w,
                height: h,
                ..Default::default()
            },
            snap: Snap::Left,
            anchor: Anchor::Left,
            is_dup: false,
            rendered: false,
            num_spaces: 0,
            force_unsnapped: false,
            is_margin_line_number: false,
            rotated: false,
            d: 0.0,
            orig_x: x,
            orig_y: y,
            orig_width: w,
            orig_height: h,
            orig_rotation: 0.0,
        }
    }

    #[test]
    fn xy_cut_finds_column_gutter_on_two_column_layout() {
        // Two columns, 50pt-wide gutter centered at x=300. Each column has
        // five rows of body text. Expect: a single vertical split at the
        // gutter, both children leaves.
        let mut items = Vec::new();
        for row in 0..5 {
            let y = 100.0 + row as f32 * 14.0;
            items.push(item_at("left side text", 50.0, y, 200.0, 10.0));
            items.push(item_at("right side text", 350.0, y, 200.0, 10.0));
        }
        let region = xy_cut(&items, 612.0, 792.0, &[]);
        match region.kind {
            RegionKind::Split { axis, children } => {
                assert_eq!(axis, CutAxis::Vertical);
                assert_eq!(children.len(), 2);
                let l = match &children[0].kind {
                    RegionKind::Leaf { item_indices } => item_indices.len(),
                    _ => 0,
                };
                let r = match &children[1].kind {
                    RegionKind::Leaf { item_indices } => item_indices.len(),
                    _ => 0,
                };
                assert_eq!(l, 5, "left col should hold 5 items");
                assert_eq!(r, 5, "right col should hold 5 items");
            }
            _ => panic!("expected a vertical split, got {:?}", region.kind),
        }
    }

    #[test]
    fn xy_cut_returns_leaf_on_single_column_prose() {
        // Tight single-column text — no valley wide enough to cut.
        let items: Vec<_> = (0..8)
            .map(|row| {
                let y = 100.0 + row as f32 * 12.0;
                item_at("the quick brown fox", 60.0, y, 400.0, 10.0)
            })
            .collect();
        let region = xy_cut(&items, 612.0, 792.0, &[]);
        assert!(matches!(region.kind, RegionKind::Leaf { .. }));
    }

    #[test]
    fn xy_cut_walk_assigns_reading_order_paths() {
        // 2-column page: pre-order walk visits left column before right.
        let mut items = Vec::new();
        for row in 0..5 {
            let y = 100.0 + row as f32 * 14.0;
            items.push(item_at("L", 50.0, y, 200.0, 10.0));
            items.push(item_at("R", 350.0, y, 200.0, 10.0));
        }
        let (lines, _region) = build_projected_lines(&items, 612.0, 792.0, &[]);
        // Left col's 5 lines come first (path [0, ...]), then right col's 5
        // (path [1, ...]).
        assert_eq!(lines.len(), 10);
        for line in &lines[..5] {
            assert_eq!(line.region_path.first().copied(), Some(0));
        }
        for line in &lines[5..] {
            assert_eq!(line.region_path.first().copied(), Some(1));
        }
    }

    #[test]
    fn banner_cut_isolates_full_width_title_above_two_columns() {
        // Layout: a centered wide title at the top, clear gap, then 2-column
        // body text. Density-based V-cut at the root would place the title
        // (centroid at page center) inside one of the columns. The banner
        // detector should H-cut just below the title so it ends up as the
        // pre-order first leaf, separate from both columns.
        let mut items = Vec::new();
        items.push(item_at(
            "Wide Centered Title Spanning Full Page",
            100.0,
            50.0,
            412.0,
            14.0,
        ));
        // Body starts well below the title (clear vertical gap).
        for row in 0..5 {
            let y = 200.0 + row as f32 * 14.0;
            items.push(item_at("left side text", 50.0, y, 200.0, 10.0));
            items.push(item_at("right side text", 350.0, y, 200.0, 10.0));
        }
        let (lines, _region) = build_projected_lines(&items, 612.0, 792.0, &[]);
        // First line (in pre-order) must be the title.
        let first = lines.first().expect("at least one line");
        assert!(
            first.text.contains("Title"),
            "expected title first, got {:?}",
            first.text
        );
        // Title leaf should be different from any column leaf.
        let body_paths: Vec<&Vec<u16>> = lines[1..].iter().map(|l| &l.region_path).collect();
        assert!(
            body_paths.iter().all(|p| **p != first.region_path),
            "title leaf should not share region_path with body lines"
        );
    }

    #[test]
    fn banner_cut_does_not_fire_on_plain_two_column_layout() {
        // Same layout as `xy_cut_finds_column_gutter_on_two_column_layout` —
        // no wide centered element to trigger a banner cut. Regression guard:
        // the V-cut must still be the first cut at the root.
        let mut items = Vec::new();
        for row in 0..5 {
            let y = 100.0 + row as f32 * 14.0;
            items.push(item_at("left side text", 50.0, y, 200.0, 10.0));
            items.push(item_at("right side text", 350.0, y, 200.0, 10.0));
        }
        let region = xy_cut(&items, 612.0, 792.0, &[]);
        match region.kind {
            RegionKind::Split { axis, .. } => {
                assert_eq!(axis, CutAxis::Vertical, "expected V-cut, not H");
            }
            _ => panic!("expected a split at the root"),
        }
    }

    #[test]
    fn figure_obstacle_forces_h_cut_around_straddling_figure() {
        // Two text bands separated by a wide figure that spans both halves of
        // the page. The text alone has no V-gutter and only a modest y-gap;
        // without obstacle seeding the root would either V-cut (bad — the
        // figure straddles both sides) or fail to cut at all. With the figure
        // stamped into density, the H-valleys above/below the figure dominate
        // and the recursion H-cuts into a top band → figure band → bottom band.
        let mut items = Vec::new();
        // Top band: several wide lines of text.
        for row in 0..4 {
            let y = 100.0 + row as f32 * 14.0;
            items.push(item_at(
                "top band line spans full width",
                50.0,
                y,
                500.0,
                12.0,
            ));
        }
        // Bottom band: several wide lines, well below the figure.
        for row in 0..4 {
            let y = 400.0 + row as f32 * 14.0;
            items.push(item_at(
                "bottom band line after the figure",
                50.0,
                y,
                500.0,
                12.0,
            ));
        }
        // Figure: 500×200pt block centered in the page between the bands.
        let figures = vec![Rect {
            x: 60.0,
            y: 160.0,
            width: 490.0,
            height: 200.0,
        }];

        let region_no_fig = xy_cut(&items, 612.0, 792.0, &[]);
        let region_with_fig = xy_cut(&items, 612.0, 792.0, &figures);

        // With figures, the root cut must be horizontal (top vs. bottom band).
        // Drilling through nested splits is fine — we just check the first one.
        match region_with_fig.kind {
            RegionKind::Split { axis, .. } => {
                assert_eq!(
                    axis,
                    CutAxis::Horizontal,
                    "figure obstacle should force an H-cut between bands"
                );
            }
            RegionKind::Leaf { .. } => panic!("expected a split when figure is present"),
        }
        // Without figures, the same layout *might* still find an H-cut (the
        // gap from y≈127 to y≈400 is real), but the cut score with figures
        // should be at least as strong. We mostly just want to confirm both
        // paths return without panicking and the figure path produces a split.
        let _ = region_no_fig;
    }

    fn has_vertical_split(region: &Region) -> bool {
        match &region.kind {
            RegionKind::Split { axis, children } => {
                *axis == CutAxis::Vertical || children.iter().any(has_vertical_split)
            }
            RegionKind::Leaf { .. } => false,
        }
    }

    fn two_column_body(items: &mut Vec<ProjectedTextItem>, y0: f32) {
        // Left column at x≈50..280, right column at x≈320..550, sharing y-bands.
        for row in 0..8 {
            let y = y0 + row as f32 * 14.0;
            items.push(item_at("left column body text here", 50.0, y, 230.0, 12.0));
            items.push(item_at(
                "right column body text here",
                320.0,
                y,
                230.0,
                12.0,
            ));
        }
    }

    #[test]
    fn spanning_line_above_columns_is_peeled_to_reveal_gutter() {
        // A single centered line that crosses the gutter (x 160..450, gutter
        // ≈300) sits above a clean two-column body. The spanning line straddles
        // the gutter so the body can't V-cut until it's peeled. The rescue in
        // the min-lines guard must allow the (finite-score) H-cut that isolates
        // the spanning line, after which the body splits into columns.
        let mut items = vec![item_at(
            "Centered Spanning Author Line",
            160.0,
            100.0,
            290.0,
            12.0,
        )];
        two_column_body(&mut items, 140.0);
        let region = xy_cut(&items, 612.0, 792.0, &[]);
        assert!(
            has_vertical_split(&region),
            "spanning line should be peeled so the two-column body V-cuts"
        );
    }

    #[test]
    fn single_column_caption_is_not_peeled_into_columns() {
        // A short centered line that sits entirely within the left column
        // (x 60..200, never crossing the gutter) must NOT trigger the spanning
        // rescue — it's a caption in running prose, not a column-straddling
        // banner. The body below it is genuinely single-column.
        let mut items = vec![item_at("Short caption", 60.0, 100.0, 140.0, 12.0)];
        for row in 0..8 {
            let y = 140.0 + row as f32 * 14.0;
            items.push(item_at(
                "single column running body text",
                50.0,
                y,
                230.0,
                12.0,
            ));
        }
        let region = xy_cut(&items, 612.0, 792.0, &[]);
        assert!(
            !has_vertical_split(&region),
            "single-column caption must not be carved into columns"
        );
    }

    #[test]
    fn project_pages_to_grid_handles_page_with_no_text_items() {
        let pages = vec![Page {
            page_number: 1,
            page_width: 612.0,
            page_height: 792.0,
            text_items: Vec::new(),
            graphics: Vec::new(),
            vector_graphics: None,
            struct_nodes: Vec::new(),
            image_refs: Vec::new(),
        }];

        let parsed = project_pages_to_grid(pages);

        assert_eq!(parsed.len(), 1);
        assert!(parsed[0].text.is_empty());
        assert!(parsed[0].text_items.is_empty());
    }

    #[test]
    fn project_pages_to_grid_unions_original_bbox_when_word_items_merge() {
        let y = 50.25;
        let pages = vec![Page {
            page_number: 1,
            page_width: 612.0,
            page_height: 792.0,
            text_items: vec![
                TextItem {
                    text: "A".to_string(),
                    x: 10.0,
                    y,
                    width: 10.0,
                    height: 8.0,
                    ..Default::default()
                },
                TextItem {
                    text: "B".to_string(),
                    x: 24.0,
                    y: y + 0.01,
                    width: 8.0,
                    height: 7.5,
                    ..Default::default()
                },
            ],
            graphics: Vec::new(),
            vector_graphics: None,
            struct_nodes: Vec::new(),
            image_refs: Vec::new(),
        }];

        let parsed = project_pages_to_grid(pages);
        let item = parsed[0]
            .text_items
            .iter()
            .find(|item| item.text == "A B")
            .expect("merged text item");

        assert!((item.x - 10.0).abs() < 0.001);
        assert!((item.y - y).abs() < 0.001);
        assert!((item.width - 22.0).abs() < 0.001);
        assert!((item.height - 8.0).abs() < 0.001);
    }

    #[test]
    fn project_pages_to_grid_concatenates_word_boxes_when_items_merge() {
        // Regression for #361: two items that merge into one must carry both
        // items' per-word sub-boxes. The item text was concatenated but the
        // second item's `words` were silently dropped, truncating `words`.
        let y = 50.25;
        let pages = vec![Page {
            page_number: 1,
            page_width: 612.0,
            page_height: 792.0,
            text_items: vec![
                TextItem {
                    text: "A".to_string(),
                    x: 10.0,
                    y,
                    width: 10.0,
                    height: 8.0,
                    words: vec![WordBox {
                        text: "A".to_string(),
                        x: 10.0,
                        y,
                        width: 10.0,
                        height: 8.0,
                    }],
                    ..Default::default()
                },
                TextItem {
                    text: "B".to_string(),
                    x: 24.0,
                    y: y + 0.01,
                    width: 8.0,
                    height: 7.5,
                    words: vec![WordBox {
                        text: "B".to_string(),
                        x: 24.0,
                        y: y + 0.01,
                        width: 8.0,
                        height: 7.5,
                    }],
                    ..Default::default()
                },
            ],
            graphics: Vec::new(),
            vector_graphics: None,
            struct_nodes: Vec::new(),
            image_refs: Vec::new(),
        }];

        let parsed = project_pages_to_grid(pages);
        let item = parsed[0]
            .text_items
            .iter()
            .find(|item| item.text == "A B")
            .expect("merged text item");

        let word_texts: Vec<&str> = item.words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(word_texts, vec!["A", "B"]);
    }

    #[test]
    fn project_pages_to_grid_unions_original_bbox_when_continuous_items_merge() {
        let pages = vec![Page {
            page_number: 1,
            page_width: 612.0,
            page_height: 792.0,
            text_items: vec![
                TextItem {
                    text: "ab".to_string(),
                    x: 40.0,
                    y: 100.0,
                    width: 10.2,
                    height: 9.0,
                    ..Default::default()
                },
                TextItem {
                    text: "cd".to_string(),
                    x: 50.2,
                    y: 100.0,
                    width: 12.3,
                    height: 9.0,
                    ..Default::default()
                },
            ],
            graphics: Vec::new(),
            vector_graphics: None,
            struct_nodes: Vec::new(),
            image_refs: Vec::new(),
        }];

        let parsed = project_pages_to_grid(pages);
        let item = parsed[0]
            .text_items
            .iter()
            .find(|item| item.text == "ab cd")
            .expect("merged continuous text item");

        assert!((item.x - 40.0).abs() < 0.001);
        assert!((item.y - 100.0).abs() < 0.001);
        assert!((item.width - 22.5).abs() < 0.01);
        assert!((item.height - 9.0).abs() < 0.01);
    }

    #[test]
    fn canonical_rotation_snaps_cardinals_and_near_cardinals() {
        // Exact cardinals are unchanged.
        assert_eq!(canonical_rotation(0.0), 0);
        assert_eq!(canonical_rotation(90.0), 90);
        assert_eq!(canonical_rotation(180.0), 180);
        assert_eq!(canonical_rotation(270.0), 270);

        // Small offsets within the 2° tolerance snap to the nearest cardinal.
        assert_eq!(canonical_rotation(1.0), 0);
        assert_eq!(canonical_rotation(88.5), 90);
        assert_eq!(canonical_rotation(271.0), 270);
    }

    #[test]
    fn canonical_rotation_snaps_near_360_to_zero() {
        // Rotations just under 360° are ~upright and must snap to 0, not be
        // treated as ~270° (regression: linear distance picked 270 for 359°).
        assert_eq!(canonical_rotation(358.0), 0);
        assert_eq!(canonical_rotation(359.0), 0);
        assert_eq!(canonical_rotation(359.5), 0);
        // rem_euclid normalizes out-of-range / negative inputs first.
        assert_eq!(canonical_rotation(360.0), 0);
        assert_eq!(canonical_rotation(-1.0), 0);
    }

    #[test]
    fn canonical_rotation_passes_through_non_cardinal_angles() {
        // Beyond the 2° snap tolerance the rounded raw angle is returned,
        // including angles near (but not within tolerance of) 360°.
        assert_eq!(canonical_rotation(45.0), 45);
        assert_eq!(canonical_rotation(357.0), 357);
    }
}
