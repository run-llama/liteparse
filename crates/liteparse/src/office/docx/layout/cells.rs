//! Cell-local picture exclusions retain source anchor origins through text reflow.

use super::*;
use crate::office::docx::model::{Anchored, HRel, VRel, Wrap};
use crate::office::docx::text::{WrapIn, layout_paragraph_resuming};

// Inline raster geometry stays source-sized; a cell-local clip bounds its paint.
// Clip items travel with each slice through row splitting and merge allocation.
pub(super) fn clip_inline_images(slices: &mut [Slice], width: f64) {
    for slice in slices {
        // Nested tables apply their own source row policy before flattening.
        if slice.nested_row.is_some() || slice.nested_table.is_some() {
            continue;
        }
        let mut items = Vec::new();
        for item in std::mem::take(&mut slice.items) {
            let clip = if let Item::Image(image) = &item {
                let mut corners = [
                    tiny_skia::Point::from_xy(0.0, 0.0),
                    tiny_skia::Point::from_xy(1.0, 0.0),
                    tiny_skia::Point::from_xy(0.0, 1.0),
                    tiny_skia::Point::from_xy(1.0, 1.0),
                ];
                image.transform.map_points(&mut corners);
                let min_y = corners
                    .iter()
                    .map(|p| p.y as f64)
                    .fold(f64::INFINITY, f64::min);
                let max_y = corners
                    .iter()
                    .map(|p| p.y as f64)
                    .fold(f64::NEG_INFINITY, f64::max);
                rect(0.0, min_y - 1.0, width, max_y - min_y + 2.0)
            } else {
                None
            };
            if let Some(path) = clip {
                items.push(Item::PushClip(path));
                items.push(item);
                items.push(Item::PopClip);
            } else {
                items.push(item);
            }
        }
        slice.items = items;
    }
}

fn local_picture(a: &Anchored, width: f64, paragraph_y: f64, line_y: f64) -> Option<(f64, f64)> {
    if a.pic.part.is_none()
        || a.pic.shape.is_some()
        || a.pic.graphic.is_some()
        || !a.text_items.is_empty()
        || !a.frame_children.is_empty()
        || a.text_frame
        || a.turned
        || a.h_rel != HRel::Column
    {
        return None;
    }
    let x = match &a.x {
        Ok(x) => *x,
        Err(s) if s == "left" => 0.0,
        Err(s) if s == "center" => (width - a.pic.w) / 2.0,
        Err(s) if s == "right" => width - a.pic.w,
        _ => return None,
    };
    let offset = *a.y.as_ref().ok()?;
    let y = match a.v_rel {
        VRel::Paragraph => paragraph_y + offset,
        VRel::Line => line_y + offset,
        _ => return None,
    };
    (x.is_finite()
        && y.is_finite()
        && offset >= 0.0
        && a.pic.w.is_finite()
        && a.pic.h.is_finite()
        && a.pic.w > 0.0
        && a.pic.h > 0.0
        && a.dist.iter().all(|d| d.is_finite()))
    .then_some((x, y))
}

fn wrapped_lines(
    p: &ParaUnit,
    bands: &[(f64, f64, f64, f64)],
    first_top: f64,
    width: f64,
    env: &TextEnv<'_>,
    consumed: usize,
) -> Option<Vec<crate::office::docx::text::Line>> {
    let rects = bands
        .iter()
        .filter(|b| b.1 > first_top + 0.01)
        .map(|b| (b.0 - first_top, b.1 - first_top, b.2, b.3))
        .collect::<Vec<_>>();
    let src = p.src.as_ref().filter(|_| {
        !rects.is_empty() && p.frame_anchors.is_empty() && p.drop_cap_anchor.is_none()
    })?;
    Some(layout_paragraph_resuming(
        src,
        width,
        env,
        &p.lines[..consumed],
        &WrapIn {
            contour: Vec::new(),
            rects,
            est_h: p.lines.first().map_or(12.0, |l| l.height),
            min_gap: 18.0,
        },
    ))
}

pub(super) fn stack(units: &[Unit], width: f64, env: &TextEnv<'_>) -> (Vec<Slice>, f64) {
    if !has_exclusion(units, width) {
        return super::stack(units);
    }
    let mut projected = projection(units.to_vec(), width, env);
    projected.resolve_inline_textboxes(&Flow::new(env, env.compat_mode));
    (projected.slices, projected.height)
}

pub(super) fn note_stack(units: &[Unit], width: f64, env: &TextEnv<'_>) -> (Vec<Slice>, f64) {
    if !has_exclusion(units, width) {
        let (mut slices, height) = super::stack(units);
        retain_paragraph_sources(units, &mut slices);
        super::note_children::retain(units, &mut slices, width, env);
        return (slices, height);
    }
    let mut projected = projection(units.to_vec(), width, env);
    projected.resolve_inline_textboxes(&Flow::new(env, env.compat_mode).for_notes());
    projected.reserve_occupied();
    retain_paragraph_sources(&projected.units, &mut projected.slices);
    super::note_children::retain(&projected.units, &mut projected.slices, width, env);
    (projected.slices, projected.height)
}

pub(super) fn retain_paragraph_sources(units: &[Unit], slices: &mut [Slice]) {
    let mut offset = 0;
    for unit in units {
        match unit {
            Unit::Para(p) => {
                let source = Rc::new(p.clone());
                for line in 0..p.lines.len() {
                    slices[offset].paragraph_source = Some((source.clone(), line));
                    offset += 1;
                }
            }
            Unit::Row(_) => offset += 1,
        }
    }
    debug_assert_eq!(offset, slices.len());
}

fn has_exclusion(units: &[Unit], width: f64) -> bool {
    units
        .iter()
        .filter_map(|u| match u {
            Unit::Para(p) => Some(p),
            _ => None,
        })
        .flat_map(|p| &p.lines)
        .flat_map(|l| &l.anchors)
        .any(|a| {
            a.wrap != Wrap::None
                && local_picture(a, width, 0.0, 0.0)
                    .is_some_and(|(x, _)| x - a.dist[2] < width && x + a.pic.w + a.dist[3] > 0.0)
        })
}

/// Retain reflowed units and their matching stacked geometry for note pagination.
pub(super) fn projection(units: Vec<Unit>, width: f64, env: &TextEnv<'_>) -> LocalProjection {
    projection_resuming(units, width, env, None)
}

pub(super) fn projection_resuming(
    units: Vec<Unit>,
    width: f64,
    env: &TextEnv<'_>,
    resume: Option<(usize, usize)>,
) -> LocalProjection {
    if !has_exclusion(&units, width) {
        let (slices, height) = super::stack(&units);
        return LocalProjection {
            units,
            slices,
            height,
            occupied: Vec::new(),
            occupied_height: height,
        };
    }
    let mut out = Vec::new();
    let mut projected = Vec::new();
    let mut occupied = Vec::new();
    let mut occupied_height: f64 = 0.0;
    let mut y = 0.0;
    let mut bands = Vec::new();
    for (unit_index, mut current) in units.into_iter().enumerate() {
        let mut extents = Vec::new();
        if let Unit::Para(p) = &mut current {
            let consumed = resume
                .filter(|(unit, _)| *unit == unit_index)
                .map_or(0, |(_, line)| line);
            let past = resume.is_some_and(|(unit, _)| unit_index < unit);
            let first_top = y + p.before + p.pad_top;
            let tail_top = first_top
                + p.lines[..consumed]
                    .iter()
                    .map(|l| l.skip + l.height)
                    .sum::<f64>();
            let paragraph_y = if consumed == 0 { y } else { tail_top };
            let mut line_y = first_top;
            let mut originals = Vec::new();
            for (line_index, line) in p.lines.iter().enumerate() {
                line_y += line.skip;
                for a in line.anchors.iter().filter(|a| !super::inline_box_owner(a)) {
                    if past || line_index < consumed {
                        originals.push((a.clone(), None));
                        continue;
                    }
                    // Earlier exclusions can move this source anchor's line. Its own
                    // exclusion is added afterward, preserving the original anchor origin.
                    let mut owner_y = line_y;
                    if a.v_rel == VRel::Line
                        && local_picture(a, width, paragraph_y, line_y).is_some()
                        && let Some(lines) =
                            wrapped_lines(p, &bands, tail_top, width, env, consumed)
                    {
                        let mut ordinal = 0;
                        let mut top = first_top;
                        for line in &lines {
                            top += line.skip;
                            if originals.len()
                                < ordinal
                                    + line
                                        .anchors
                                        .iter()
                                        .filter(|a| !super::inline_box_owner(a))
                                        .count()
                            {
                                owner_y = top;
                                break;
                            }
                            ordinal += line
                                .anchors
                                .iter()
                                .filter(|a| !super::inline_box_owner(a))
                                .count();
                            top += line.height;
                        }
                    }
                    let position = local_picture(a, width, paragraph_y, owner_y);
                    if let Some((x, ay)) = position
                        && a.wrap != Wrap::None
                        && x - a.dist[2] < width
                        && x + a.pic.w + a.dist[3] > 0.0
                    {
                        if a.wrap == Wrap::TopAndBottom {
                            bands.push((ay - a.dist[0], ay + a.pic.h + a.dist[1], 0.0, width));
                        } else {
                            bands.extend(crate::office::docx::paginate::wrap_boxes(
                                a, x, ay, a.pic.w, a.pic.h,
                            ));
                        }
                    }
                    originals.push((a.clone(), position));
                }
                line_y += line.height;
            }
            if let Some(lines) = wrapped_lines(p, &bands, tail_top, width, env, consumed) {
                p.lines = lines;
            }
            let mut original = originals.into_iter();
            line_y = first_top;
            for line in &mut p.lines {
                line_y += line.skip;
                let mut extent = line.height;
                for a in line
                    .anchors
                    .iter_mut()
                    .filter(|a| !super::inline_box_owner(a))
                {
                    if let Some((mut source, position)) = original.next() {
                        if let Some((x, ay)) = position {
                            if source.wrap != Wrap::None
                                && x - source.dist[2] < width
                                && x + source.pic.w + source.dist[3] > 0.0
                            {
                                extent = extent.max(ay + source.pic.h + source.dist[1] - line_y);
                            }
                            source.x = Ok(x);
                            source.v_rel = VRel::Line;
                            source.y = Ok(ay - line_y);
                        }
                        *a = source;
                    }
                }
                occupied_height = occupied_height.max(line_y + extent);
                extents.push(extent);
                line_y += line.height;
            }
        }
        let (mut slices, height) =
            super::stack_with_clearance(std::slice::from_ref(&current), true);
        for slice in &mut slices {
            slice.y += y;
            for item in &mut slice.items {
                translate_item(item, 0.0, y);
            }
            for anchor in &mut slice.anchors {
                anchor.2 += y;
            }
        }
        occupied.push(extents);
        projected.push(current);
        out.extend(slices);
        y += height;
    }
    LocalProjection {
        units: projected,
        slices: out,
        height: y,
        occupied,
        occupied_height: occupied_height.max(y),
    }
}

#[cfg(test)]
mod tests;
