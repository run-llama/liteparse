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

    let mid = widths.len() / 2;

    let median_width = if widths.len().is_multiple_of(2) {
        (widths[mid - 1] + widths[mid]) / 2.0
    } else {
        widths[mid]
    };

    let median_height = if heights.len().is_multiple_of(2) {
        (heights[mid - 1] + heights[mid]) / 2.0
    } else {
        heights[mid]
    };

    (median_width, median_height)
}

fn canonical_rotation(rotation: f32) -> i32 {
    let r = rotation.rem_euclid(360.0);

    let candidates = [0.0f32, 90.0, 180.0, 270.0];
    let mut best = 0.0f32;
    let mut best_delta = f32::INFINITY;
    for c in candidates {
        let delta = (r - c).abs();
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

        if global_overlap {
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

    // Merge continuous bbox items in a single linear pass.
    let merge_y_tolerance = 0.1;
    let merge_h_tolerance = 0.1;

    let mut merged_items: Vec<ProjectedTextItem> = Vec::with_capacity(items.len());
    for cur in items.drain(..) {
        let should_merge = merged_items
            .last()
            .map(|prev| can_merge(prev, &cur, merge_y_tolerance, merge_h_tolerance))
            .unwrap_or(false);

        if should_merge {
            if let Some(prev) = merged_items.last_mut() {
                let merged = merge_bbox(prev, &cur);
                prev.item.text.push_str(&cur.item.text);
                prev.item.x = merged.0;
                prev.item.y = merged.1;
                prev.item.width = merged.2;
                prev.item.height = merged.3;
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
            let too_tall = (proposed_max_y - proposed_min_y) > median_height * 1.8;

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

    // merge 'words'
    const MERGE_THRESHOLD: f32 = 1.0;

    fn looks_like_table_number(text: &str) -> bool {
        let trimmed = text.trim();
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
        for item in line.drain(..) {
            if let Some(prev) = merged_line.last_mut() {
                let both_are_numbers = looks_like_table_number(&prev.item.text)
                    && looks_like_table_number(&item.item.text);

                let delta_x = item.item.x - prev.item.x - prev.item.width;
                // Don't merge items with noticeably different y positions (>1.5px).
                // Items on the same baseline typically differ by <0.5px.
                let y_diff = (item.item.y - prev.item.y).abs();
                let y_compatible = y_diff <= 1.5;

                if y_compatible && !both_are_numbers && delta_x <= MERGE_THRESHOLD {
                    prev.item.width = item.item.x + item.item.width - prev.item.x;
                    prev.item.text.push_str(&item.item.text);
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
                let mut current = lines.remove(i);
                lines[i - 1].append(&mut current);
                lines[i - 1].sort_by(|a, b| a.item.x.total_cmp(&b.item.x));
                continue;
            }
        }

        i += 1;
    }

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

fn median_f32(values: &mut [f32]) -> Option<f32> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.total_cmp(b));
    Some(values[values.len() / 2])
}

fn line_bounds(line: &[ProjectedTextItem]) -> Option<(f32, f32)> {
    if line.is_empty() {
        return None;
    }
    let min_x = line
        .iter()
        .map(|item| item.item.x)
        .fold(f32::INFINITY, f32::min);
    let max_x = line
        .iter()
        .map(|item| item.item.x + item.item.width)
        .fold(f32::NEG_INFINITY, f32::max);
    Some((min_x, max_x))
}

fn line_text_len(line: &[ProjectedTextItem]) -> usize {
    line.iter().map(|item| item.item.text.chars().count()).sum()
}

fn best_column_gap(
    line: &[ProjectedTextItem],
    median_width: f32,
    page_width: f32,
) -> Option<(usize, f32)> {
    if line.len() < 2 {
        return None;
    }

    let midpoint = page_width * 0.5;
    let mut best: Option<(usize, f32, f32)> = None;
    for i in 1..line.len() {
        let prev_end = line[i - 1].item.x + line[i - 1].item.width;
        let cur_start = line[i].item.x;
        let gap = cur_start - prev_end;
        if gap > median_width * 2.0 && prev_end < midpoint && cur_start > midpoint {
            let split_x = (prev_end + cur_start) * 0.5;
            if best
                .as_ref()
                .map_or(true, |(_, best_gap, _)| gap > *best_gap)
            {
                best = Some((i, gap, split_x));
            }
        }
    }

    best.map(|(idx, _, split_x)| (idx, split_x))
}

fn detect_two_column_split(
    run: &[Vec<ProjectedTextItem>],
    median_width: f32,
    page_width: f32,
) -> Option<f32> {
    if page_width <= 0.0 {
        return None;
    }

    let non_empty_lines = run.iter().filter(|line| !line.is_empty()).count();
    if non_empty_lines < 6 {
        return None;
    }

    let mut split_candidates = Vec::new();
    let mut left_spans = Vec::new();
    let mut right_spans = Vec::new();
    let mut left_text_lens = Vec::new();
    let mut right_text_lens = Vec::new();

    for line in run {
        let Some((split_idx, split_x)) = best_column_gap(line, median_width, page_width) else {
            continue;
        };
        let left = &line[..split_idx];
        let right = &line[split_idx..];
        let Some((left_min, left_max)) = line_bounds(left) else {
            continue;
        };
        let Some((right_min, right_max)) = line_bounds(right) else {
            continue;
        };

        split_candidates.push(split_x);
        left_spans.push(left_max - left_min);
        right_spans.push(right_max - right_min);
        left_text_lens.push(line_text_len(left) as f32);
        right_text_lens.push(line_text_len(right) as f32);
    }

    let column_gap_lines = split_candidates.len();
    if column_gap_lines < 3 {
        return None;
    }
    if column_gap_lines < 5 && column_gap_lines * 4 < non_empty_lines {
        return None;
    }

    let left_span = median_f32(&mut left_spans)?;
    let right_span = median_f32(&mut right_spans)?;
    if left_span < page_width * 0.2 || right_span < page_width * 0.2 {
        return None;
    }

    let left_len = median_f32(&mut left_text_lens)?;
    let right_len = median_f32(&mut right_text_lens)?;
    if left_len < 12.0 || right_len < 12.0 {
        return None;
    }

    median_f32(&mut split_candidates)
}

fn split_line_by_column(
    line: Vec<ProjectedTextItem>,
    split_x: f32,
) -> (
    Vec<ProjectedTextItem>,
    Vec<ProjectedTextItem>,
    Option<Vec<ProjectedTextItem>>,
) {
    let Some((line_min, line_max)) = line_bounds(&line) else {
        return (Vec::new(), Vec::new(), None);
    };

    let crosses_split = line_min < split_x && line_max > split_x;
    let has_item_crossing_split = line
        .iter()
        .any(|item| item.item.x < split_x && item.item.x + item.item.width > split_x);
    if crosses_split && has_item_crossing_split {
        return (Vec::new(), Vec::new(), Some(line));
    }

    let mut left = Vec::new();
    let mut right = Vec::new();
    for item in line {
        let center = item.item.x + item.item.width * 0.5;
        if center < split_x {
            left.push(item);
        } else {
            right.push(item);
        }
    }

    (left, right, None)
}

fn append_reflowed_columns(
    output: &mut Vec<Vec<ProjectedTextItem>>,
    left_lines: &mut Vec<Vec<ProjectedTextItem>>,
    right_lines: &mut Vec<Vec<ProjectedTextItem>>,
) {
    normalize_right_column_origin(left_lines, right_lines);
    output.append(left_lines);
    output.append(right_lines);
}

fn min_x_for_lines(lines: &[Vec<ProjectedTextItem>]) -> Option<f32> {
    lines
        .iter()
        .flat_map(|line| line.iter().map(|item| item.item.x))
        .min_by(|a, b| a.total_cmp(b))
}

fn normalize_right_column_origin(
    left_lines: &[Vec<ProjectedTextItem>],
    right_lines: &mut [Vec<ProjectedTextItem>],
) {
    let Some(left_min_x) = min_x_for_lines(left_lines) else {
        return;
    };
    let Some(right_min_x) = min_x_for_lines(right_lines) else {
        return;
    };
    let offset = right_min_x - left_min_x;
    if offset <= 0.0 {
        return;
    }

    for line in right_lines {
        for item in line {
            item.item.x -= offset;
        }
    }
}

fn reflow_two_column_run(
    run: Vec<Vec<ProjectedTextItem>>,
    split_x: f32,
) -> Vec<Vec<ProjectedTextItem>> {
    let mut output = Vec::with_capacity(run.len());
    let mut left_lines = Vec::new();
    let mut right_lines = Vec::new();

    for line in run {
        let (left, right, full_width) = split_line_by_column(line, split_x);
        if let Some(full_width) = full_width {
            append_reflowed_columns(&mut output, &mut left_lines, &mut right_lines);
            output.push(full_width);
            continue;
        }
        if !left.is_empty() {
            left_lines.push(left);
        }
        if !right.is_empty() {
            right_lines.push(right);
        }
    }

    append_reflowed_columns(&mut output, &mut left_lines, &mut right_lines);
    output
}

fn reflow_multicolumn_text_runs(
    lines: Vec<Vec<ProjectedTextItem>>,
    median_width: f32,
    page_width: f32,
) -> Vec<Vec<ProjectedTextItem>> {
    let mut output = Vec::with_capacity(lines.len());
    let mut run = Vec::new();

    for line in lines {
        if line.is_empty() {
            if let Some(split_x) = detect_two_column_split(&run, median_width, page_width) {
                output.extend(reflow_two_column_run(std::mem::take(&mut run), split_x));
            } else {
                output.append(&mut run);
            }
            output.push(line);
        } else {
            run.push(line);
        }
    }

    if let Some(split_x) = detect_two_column_split(&run, median_width, page_width) {
        output.extend(reflow_two_column_run(run, split_x));
    } else {
        output.extend(run);
    }

    output
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

    for i in 0..line.len() {
        if i > 0 {
            let prev = &line[i - 1].item;
            let cur = &line[i].item;
            let gap = cur.x - (prev.x + prev.width);
            let space_threshold =
                (cur.height * FLOWING_SPACE_HEIGHT_RATIO).max(FLOWING_SPACE_MIN_THRESHOLD);
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
    lines = reflow_multicolumn_text_runs(lines, median_width, page.page_width);
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
            item.item.x = meta[line_idx][box_idx].projected_x as f32;
            item.item.y = line_idx as f32;
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
            ParsedPage {
                page_number: page.page_number,
                page_width: page.page_width,
                page_height: page.page_height,
                text,
                text_items: projected_items.into_iter().map(|proj| proj.item).collect(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_item(text: &str, x: f32, y: f32, width: f32) -> TextItem {
        TextItem {
            text: text.into(),
            x,
            y,
            width,
            height: 10.0,
            ..Default::default()
        }
    }

    fn project_text(items: Vec<TextItem>) -> String {
        let page = Page {
            page_number: 1,
            page_width: 600.0,
            page_height: 800.0,
            text_items: items.clone(),
        };
        let projection_boxes = items
            .into_iter()
            .map(|item| ProjectedTextItem {
                item,
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

        let (_, text) = project_to_grid(&page, projection_boxes);
        text
    }

    #[test]
    fn reflows_prose_two_column_runs_in_reading_order() {
        let items = vec![
            text_item("left column first paragraph line one", 60.0, 100.0, 220.0),
            text_item("right column first paragraph line one", 330.0, 100.0, 220.0),
            text_item("left column first paragraph line two", 60.0, 112.0, 220.0),
            text_item("right column first paragraph line two", 330.0, 112.0, 220.0),
            text_item("left column first paragraph line three", 60.0, 124.0, 220.0),
            text_item(
                "right column first paragraph line three",
                330.0,
                124.0,
                220.0,
            ),
            text_item("left column continuation line four", 60.0, 136.0, 220.0),
            text_item("left column continuation line five", 60.0, 148.0, 220.0),
            text_item("left column continuation line six", 60.0, 160.0, 220.0),
        ];

        let text = project_text(items);

        assert!(
            text.find("left column continuation line six").unwrap()
                < text.find("right column first paragraph line one").unwrap()
        );
        let right_line = text
            .lines()
            .find(|line| line.contains("right column first paragraph line one"))
            .unwrap();
        assert!(right_line.len() - right_line.trim_start().len() <= 4);
        assert!(!text.lines().any(|line| {
            line.contains("left column first") && line.contains("right column first")
        }));
    }

    #[test]
    fn keeps_isolated_two_column_rows_visual() {
        let items = vec![
            text_item("body line before footer", 60.0, 100.0, 220.0),
            text_item("414", 60.0, 130.0, 30.0),
            text_item(
                "Japan Marketing Academy Conference Proceedings",
                330.0,
                130.0,
                220.0,
            ),
        ];

        let text = project_text(items);

        assert!(text.lines().any(|line| {
            line.contains("414") && line.contains("Japan Marketing Academy Conference Proceedings")
        }));
    }
}
