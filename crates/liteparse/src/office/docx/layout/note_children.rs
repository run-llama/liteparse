//! Recursive table fragments retain source cursors until final note painting.

use super::*;
use crate::office::docx::paginate::note_fragments::{
    FragmentBudget, FragmentPlan, NoteCursor, NoteLayout, NotePlacement, plan_fragment_in_env,
};

// The immutable source and accepted cursor travel with an enclosing cell slice.
// Cached drawing items are refreshed after all fragments of this occurrence exist.
#[derive(Clone)]
pub struct ChildTable {
    pub layout: NoteLayout,
    pub before: NoteCursor,
    pub plan: FragmentPlan,
    pub text_keys: Vec<usize>,
    pub anchor_keys: Vec<(usize, f64)>,
}

impl std::fmt::Debug for ChildTable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChildTable")
            .field("height", &self.plan.used_height)
            .finish_non_exhaustive()
    }
}

// A remaining-source preview supplies cached geometry for the enclosing splitter.
// Physical acceptance uses the same planner with the actual inner-cell budget.
fn preview(
    layout: &NoteLayout,
    before: &NoteCursor,
    env: Option<&TextEnv<'_>>,
) -> Option<FragmentPlan> {
    let plan = plan_fragment_in_env(
        layout,
        before,
        FragmentBudget {
            height: 1.0e12,
            fresh_capacity: 1.0e12,
            fresh: true,
            allow_atomic_overflow: true,
        },
        env,
    )
    .ok()?;
    plan.after.complete().then_some(plan)
}

pub(super) fn retain(units: &[Unit], slices: &mut Vec<Slice>, width: f64, env: &TextEnv<'_>) {
    let mut unit = 0;
    let mut slice = 0;
    while unit < units.len() {
        match &units[unit] {
            Unit::Para(p) => {
                slice += p.lines.len();
                unit += 1;
            }
            Unit::Row(row) => {
                let mut end = unit + 1;
                while matches!(units.get(end), Some(Unit::Row(next)) if next.table == row.table) {
                    end += 1;
                }
                let merged = units[unit..end].iter().any(
                    |u| matches!(u, Unit::Row(r) if r.cells.iter().any(|c| c.moved.is_some())),
                );
                if merged {
                    let layout =
                        NoteLayout::with_local_pictures(units[unit..end].to_vec(), width, env);
                    let before = layout.start();
                    if let Some(plan) = preview(&layout, &before, Some(env)) {
                        let child = Rc::new(ChildTable {
                            layout,
                            before,
                            plan,
                            text_keys: Vec::new(),
                            anchor_keys: Vec::new(),
                        });
                        let y = slices[slice].y;
                        slices.splice(slice..slice + end - unit, [materialize(child, y, &[])]);
                        slice += 1;
                    } else {
                        slice += end - unit;
                    }
                } else {
                    slice += end - unit;
                }
                unit = end;
            }
        }
    }
}

pub(crate) fn remeasure(
    child: &ChildTable,
    width: f64,
    env: &TextEnv<'_>,
    y: f64,
) -> Option<Slice> {
    let (layout, before) = child.layout.remeasure(&child.before, width, env)?;
    let plan = preview(&layout, &before, Some(env))?;
    Some(materialize(
        Rc::new(ChildTable {
            layout,
            before,
            plan,
            text_keys: Vec::new(),
            anchor_keys: Vec::new(),
        }),
        y,
        &[],
    ))
}

pub(crate) fn split(
    s: &Slice,
    limit: f64,
    every_cell: bool,
    fresh_capacity: f64,
    env: Option<&TextEnv<'_>>,
) -> Option<(Slice, Slice)> {
    let child = s.nested_table.as_ref()?;
    let height = (limit - s.y).min(fresh_capacity).max(0.0);
    let plan = plan_fragment_in_env(
        &child.layout,
        &child.before,
        FragmentBudget {
            height,
            fresh_capacity,
            fresh: !every_cell && s.y <= 0.01,
            allow_atomic_overflow: false,
        },
        env,
    )
    .ok()?;
    if plan.placements.is_empty() || plan.after.complete() {
        return None;
    }
    let after = plan.after.clone();
    let top_height = plan.used_height;
    let top = materialize(
        Rc::new(ChildTable {
            layout: child.layout.clone(),
            before: child.before.clone(),
            plan,
            text_keys: Vec::new(),
            anchor_keys: Vec::new(),
        }),
        s.y,
        &[],
    );
    let rest_plan = preview(&child.layout, &after, env)?;
    let rest = materialize(
        Rc::new(ChildTable {
            layout: child.layout.clone(),
            before: after,
            plan: rest_plan,
            text_keys: Vec::new(),
            anchor_keys: Vec::new(),
        }),
        s.y + top_height,
        &[],
    );
    Some((top, rest))
}

// Final painting gathers earlier destinations from the same child occurrence.
// Geometry and source progress were already accepted by the parent transaction.
pub(crate) fn materialize(child: Rc<ChildTable>, y: f64, accepted: &[Rc<ChildTable>]) -> Slice {
    let mut items = Vec::new();
    let mut anchors = Vec::new();
    let mut notes = Vec::new();
    let mut text_keys = Vec::new();
    let mut anchor_keys = Vec::new();
    let stride = plans_stride(&child, accepted);
    let mut plans = accepted
        .iter()
        .filter(|other| other.before.same_occurrence(&child.before))
        .cloned()
        .collect::<Vec<_>>();
    if plans.is_empty() {
        plans.push(child.clone());
    }
    for placement in &child.plan.placements {
        let NotePlacement::Row {
            row,
            y: row_y,
            history,
            ..
        } = placement
        else {
            continue;
        };
        let row = materialize_row(row, accepted);
        text_keys.extend(std::iter::repeat_n(
            row.index * 1024 * stride,
            row.bookmarks_before.len(),
        ));
        for (ci, cell) in row.cells.iter().enumerate() {
            let base = (row.index * 1024 + ci) * stride;
            let mut slot = 0;
            let mut text = Vec::new();
            let mut floats = Vec::new();
            for slice in cell.slices.iter().chain(cell.moved_slices()) {
                crate::office::docx::paginate::note_areas::note_slice_keys(
                    slice,
                    &mut slot,
                    &mut text,
                    &mut floats,
                );
            }
            text_keys.extend(text.into_iter().map(|key| base + key));
            anchor_keys.extend(floats.into_iter().map(|(key, offset)| (base + key, offset)));
        }
        text_keys.extend(std::iter::repeat_n(
            (row.index * 1024 + 1023) * stride,
            row.bookmarks_after.len(),
        ));
        items.extend(row_items(&row, 0.0, y + row_y + row.outer_before));
        anchors.extend(row_anchors(&row, 0.0, y + row_y + row.outer_before));
        notes.extend(row.notes());
        for plan in &plans {
            for placement in &plan.plan.placements {
                let NotePlacement::Row { merge_paint, .. } = placement else {
                    continue;
                };
                for original in merge_paint.iter().filter(|p| p.owner == *history) {
                    let mut paint = original.clone();
                    materialize_paint(&mut paint, accepted);
                    let base = (paint.restart.1 * 1024 + paint.cell) * stride;
                    for item in &paint.items {
                        if let Some(key) = crate::office::docx::text::reading_key(item) {
                            text_keys.push(base + key);
                        }
                        let mut item = item.clone();
                        translate_item(&mut item, 0.0, y);
                        items.push(item);
                    }
                    anchor_keys.extend(paint.anchor_keys.iter().map(|key| (base + key, 0.0)));
                    anchors.extend(
                        paint
                            .anchors
                            .iter()
                            .map(|(a, x, ay)| (a.clone(), *x, y + ay)),
                    );
                    notes.extend(paint.notes.iter().copied());
                }
            }
        }
    }
    let mut ranks = text_keys
        .iter()
        .copied()
        .chain(anchor_keys.iter().map(|(key, _)| *key))
        .collect::<Vec<_>>();
    ranks.sort_unstable();
    ranks.dedup();
    for key in &mut text_keys {
        *key = ranks.binary_search(key).unwrap();
    }
    for (key, _) in &mut anchor_keys {
        *key = ranks.binary_search(key).unwrap();
    }
    let mut child = child.as_ref().clone();
    child.text_keys = text_keys;
    child.anchor_keys = anchor_keys;
    let child = Rc::new(child);
    Slice {
        y,
        h: child.plan.used_height,
        items,
        anchors,
        notes,
        paragraph_offset: 0.0,
        widow: None,
        nested_row: None,
        nested_table: Some(child),
        paragraph_source: None,
    }
}

pub(crate) fn materialize_row(row: &RowUnit, accepted: &[Rc<ChildTable>]) -> RowUnit {
    let mut row = row.clone();
    for cell in &mut row.cells {
        for slice in &mut cell.slices {
            if let Some(child) = &slice.nested_table {
                *slice = materialize(child.clone(), slice.y, accepted);
            } else if let Some(nested) = &slice.nested_row {
                *slice =
                    super::nested_row_slice(Rc::new(materialize_row(nested, accepted)), slice.y);
            }
        }
    }
    row
}

pub(crate) fn collect(row: &RowUnit, out: &mut Vec<Rc<ChildTable>>) {
    for cell in &row.cells {
        for slice in cell.slices.iter().chain(cell.moved_slices()) {
            if let Some(child) = &slice.nested_table {
                out.push(child.clone());
                for placement in &child.plan.placements {
                    if let NotePlacement::Row {
                        row, merge_paint, ..
                    } = placement
                    {
                        collect(row, out);
                        for paint in merge_paint {
                            collect_paint(paint, out);
                        }
                    }
                }
            }
            if let Some(nested) = &slice.nested_row {
                collect(nested, out);
            }
        }
    }
}

fn plans_stride(child: &ChildTable, accepted: &[Rc<ChildTable>]) -> usize {
    std::iter::once(child)
        .chain(accepted.iter().map(Rc::as_ref))
        .filter(|other| other.before.same_occurrence(&child.before))
        .map(|other| other.layout.reading_stride())
        .max()
        .unwrap_or(1)
}

// Deferred merged payload keeps accepted child cursors for final backfill.
// The stored translation is the original accepted destination, in points.
pub(crate) fn collect_paint(
    paint: &crate::office::docx::paginate::merges::Paint,
    out: &mut Vec<Rc<ChildTable>>,
) {
    if let Some(retained) = &paint.retained {
        for slice in &retained.slices {
            collect_slice(slice, out);
        }
    }
}

fn collect_slice(slice: &Slice, out: &mut Vec<Rc<ChildTable>>) {
    if let Some(child) = &slice.nested_table {
        out.push(child.clone());
        for placement in &child.plan.placements {
            if let NotePlacement::Row {
                row, merge_paint, ..
            } = placement
            {
                collect(row, out);
                for paint in merge_paint {
                    collect_paint(paint, out);
                }
            }
        }
    }
    if let Some(row) = &slice.nested_row {
        collect(row, out);
    }
}

pub(crate) fn materialize_paint(
    paint: &mut crate::office::docx::paginate::merges::Paint,
    accepted: &[Rc<ChildTable>],
) {
    let Some(retained) = &paint.retained else {
        return;
    };
    let mut slot = retained.slot;
    let mut items = Vec::new();
    let mut anchors = Vec::new();
    let mut anchor_keys = Vec::new();
    let mut notes = Vec::new();
    for original in &retained.slices {
        let slice = if let Some(child) = &original.nested_table {
            materialize(child.clone(), original.y, accepted)
        } else if let Some(row) = &original.nested_row {
            super::nested_row_slice(Rc::new(materialize_row(row, accepted)), original.y)
        } else {
            original.clone()
        };
        let mut keys = Vec::new();
        let mut float_keys = Vec::new();
        crate::office::docx::paginate::note_areas::note_slice_keys(
            &slice,
            &mut slot,
            &mut keys,
            &mut float_keys,
        );
        let mut keys = keys.into_iter();
        for mut item in slice.items {
            if let Some(key) = crate::office::docx::text::reading_key_mut(&mut item) {
                *key = keys
                    .next()
                    .expect("retained text slots match accepted source");
            }
            translate_item(&mut item, retained.x, retained.y);
            items.push(item);
        }
        anchor_keys.extend(float_keys.into_iter().map(|(key, _)| key));
        anchors.extend(
            slice
                .anchors
                .into_iter()
                .map(|(a, x, y)| (a, x + retained.x, y + retained.y)),
        );
        notes.extend(slice.notes);
    }
    super::stamp_fragment_source(&mut items, paint.fragment_source.as_deref());
    paint.items = items;
    paint.anchors = anchors;
    paint.anchor_keys = anchor_keys;
    paint.notes = notes;
}
