use super::{CellDefinition, decode_table_definition};
use crate::office::doc::{
    Error, Inspection, Result, bytes,
    formatting_inspection::FormattingInspection,
    paragraph_formatting::ParagraphInspection,
    structure::{ParagraphOwnership, StructurePlan},
    u16_at, u32_at,
};
use serde::Serialize;
const MAX_ITEMS: usize = 262_144;
const MAX_DEPTH: usize = 64;

#[derive(Debug, Serialize)]
pub struct TableOwnership {
    pub status: &'static str,
    pub stories: Vec<TableStory>,
    pub unresolved: [&'static str; 3],
}
#[derive(Debug, Serialize)]
pub struct TableStory {
    pub kind: &'static str,
    pub rows: Vec<OwnedRow>,
    pub outside_paragraphs: Vec<usize>,
    pub unresolved_ranges: Vec<UnresolvedRange>,
    pub paragraph_contexts: Vec<ParagraphContext>,
}
#[derive(Debug, Serialize)]
pub struct ParagraphContext {
    pub paragraph_index: usize,
    pub paragraph_group_id: Option<u32>,
    pub frame_anchor_locked: Option<bool>,
    pub unresolved: Vec<&'static str>,
}
#[derive(Debug, Serialize)]
pub struct UnresolvedRange {
    pub paragraph_range: [usize; 2],
    pub reason: &'static str,
}
#[derive(Debug, Serialize)]
pub struct OwnedRow {
    pub depth: usize,
    pub source_range: [usize; 2],
    pub projected_bytes: Option<[usize; 2]>,
    pub row_mark_paragraph: usize,
    pub cells: Vec<OwnedCell>,
    pub definition_cell_count: Option<usize>,
    pub unresolved: Vec<&'static str>,
}
#[derive(Debug, Serialize)]
pub struct OwnedCell {
    pub source_range: [usize; 2],
    pub projected_bytes: Option<[usize; 2]>,
    pub content: Vec<CellContent>,
    pub definition: Option<CellDefinition>,
}
#[derive(Debug, Serialize)]
#[serde(tag = "kind", content = "index")]
pub enum CellContent {
    Paragraph(usize),
    NestedRow(usize),
}
#[derive(Default)]
struct State {
    depth: usize,
    cell: bool,
    row: bool,
    definitions: Vec<CellDefinition>,
    definition_known: bool,
    revision: bool,
    paragraph_group_id: Option<u32>,
    frame_anchor_locked: Option<bool>,
}
struct Frame {
    start: usize,
    cell_start: usize,
    cells: Vec<OwnedCell>,
    content: Vec<CellContent>,
    revision: bool,
}

/// Preserve innermost paragraph ownership and nested-row order without copying cell text.
/// Source cell marks survive merges; the plan does not infer visible merge owners or table groups.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/5b45f0e7-7760-4fdb-af88-0146de2feb4c>
pub fn plan_tables(
    source: &Inspection,
    structure: &StructurePlan,
    paragraphs: &ParagraphInspection,
    formatting: &FormattingInspection,
) -> Result<TableOwnership> {
    if source.binary_version != paragraphs.binary_version
        || source.binary_version != formatting.binary_version
        || source.stories.len() != structure.stories.len()
        || source.stories.len() > 8
        || paragraphs.spans.len() > MAX_ITEMS
        || formatting.pieces.len() > MAX_ITEMS
    {
        return Err(Error::Invalid("table metadata identity"));
    }
    if paragraphs
        .spans
        .windows(2)
        .any(|w| w[0].end_cp != w[1].start_cp)
        || paragraphs.spans.iter().any(|p| p.start_cp >= p.end_cp)
        || formatting
            .pieces
            .windows(2)
            .any(|w| w[0].end_cp != w[1].start_cp)
        || formatting.pieces.iter().any(|p| p.start_cp >= p.end_cp)
    {
        return Err(Error::Invalid("table source span order"));
    }
    let mut source_units = 16 * 1024 * 1024usize;
    let mut source_bytes = 64 * 1024 * 1024usize;
    for story in &source.stories {
        source_units = source_units
            .checked_sub(story.utf16_units)
            .ok_or(Error::Limit("table source units"))?;
        source_bytes = source_bytes
            .checked_sub(story.text.len())
            .ok_or(Error::Limit("table source bytes"))?;
        if story.text.encode_utf16().count() != story.utf16_units {
            return Err(Error::Invalid("table source text length"));
        }
    }
    let mut work = 8 * 1024 * 1024usize;
    let mut item_budget = MAX_ITEMS;
    let mut definition_budget = MAX_ITEMS;
    let mut stories = Vec::new();
    for (raw, part) in source.stories.iter().zip(&structure.stories) {
        if raw.kind != part.kind {
            return Err(Error::Invalid("table story identity"));
        }
        let mut cp = raw.start_cp;
        for p in &part.paragraphs {
            if p.source_range[0] != cp || p.source_range[1] <= cp {
                return Err(Error::Invalid("table paragraph order"));
            }
            if p.projected_bytes.is_some_and(|b| b[0] > b[1])
                || p.endpoint
                    .as_ref()
                    .is_some_and(|e| e.end_cp != p.source_range[1])
            {
                return Err(Error::Invalid("table paragraph provenance"));
            }
            cp = p.source_range[1];
        }
        if raw.start_cp.checked_add(raw.utf16_units) != Some(cp) {
            return Err(Error::Invalid("table paragraph coverage"));
        }
        let mut states = Vec::new();
        for p in &part.paragraphs {
            item_budget = item_budget
                .checked_sub(1)
                .ok_or(Error::Limit("table paragraph count"))?;
            let mark = p.source_range[1] - 1;
            let pap = paragraphs
                .spans
                .get(paragraphs.spans.partition_point(|p| p.end_cp <= mark))
                .filter(|p| p.start_cp <= mark && p.end_cp == mark + 1);
            let piece = formatting
                .pieces
                .get(formatting.pieces.partition_point(|p| p.end_cp <= mark))
                .filter(|p| p.start_cp <= mark);
            let state = match (pap, piece, p.endpoint.as_ref()) {
                (Some(pap), Some(piece), Some(_))
                    if matches!(p.terminal_unit, Some(7 | 12 | 13)) =>
                {
                    resolve(
                        &pap.raw_modifiers,
                        piece.raw_prm,
                        &formatting.raw_property_groups,
                        p.terminal_unit,
                        source.binary_version,
                        &mut work,
                    )
                }
                _ => Err(Error::Unsupported("table paragraph endpoint metadata")),
            };
            match state {
                Ok(s) => {
                    charge_definitions(&mut definition_budget, s.definitions.len())?;
                    states.push(Some(s));
                }
                Err(Error::Unsupported(_)) => states.push(None),
                Err(e) => return Err(e),
            }
        }
        stories.push(assemble(part.kind, &part.paragraphs, states)?);
    }
    Ok(TableOwnership {
        status: "partial_source_cell_row_ownership",
        stories,
        unresolved: [
            "adjacent_row_table_grouping",
            "merged_cell_visible_content_ownership",
            "revised_or_hidden_table_topology",
        ],
    })
}
fn charge_definitions(remaining: &mut usize, count: usize) -> Result<()> {
    *remaining = remaining
        .checked_sub(count)
        .ok_or(Error::Limit("table definition records"))?;
    Ok(())
}
fn range(
    paragraphs: &[ParagraphOwnership],
    start: usize,
    end: usize,
) -> ([usize; 2], Option<[usize; 2]>) {
    let first = &paragraphs[start];
    let last = &paragraphs[end - 1];
    let projected = (|| {
        let initial = first.projected_bytes?;
        let final_bytes = last.projected_bytes?;
        let mut known_end = initial[0];
        for paragraph in &paragraphs[start..end] {
            if let Some(bytes) = paragraph.projected_bytes {
                if bytes[0] < known_end || bytes[1] < bytes[0] || bytes[1] > final_bytes[1] {
                    return None;
                }
                known_end = bytes[1];
            }
        }
        Some([initial[0], final_bytes[1]])
    })();
    ([first.source_range[0], last.source_range[1]], projected)
}
fn flush(stack: &mut Vec<Frame>, end: usize, out: &mut TableStory, reason: &'static str) {
    if let Some(frame) = stack.first() {
        out.unresolved_ranges.push(UnresolvedRange {
            paragraph_range: [frame.start, end],
            reason,
        });
    }
    stack.clear();
}
fn assemble(
    kind: &'static str,
    paragraphs: &[ParagraphOwnership],
    states: Vec<Option<State>>,
) -> Result<TableStory> {
    let mut out = TableStory {
        kind,
        rows: Vec::new(),
        outside_paragraphs: Vec::new(),
        unresolved_ranges: Vec::new(),
        paragraph_contexts: Vec::new(),
    };
    let mut stack: Vec<Frame> = Vec::new();
    let mut root_start_known = true;
    for (index, state) in states.into_iter().enumerate() {
        let Some(state) = state else {
            root_start_known = false;
            flush(&mut stack, index, &mut out, "table_context_unresolved");
            out.unresolved_ranges.push(UnresolvedRange {
                paragraph_range: [index, index + 1],
                reason: "table_context_unresolved",
            });
            continue;
        };
        if state.paragraph_group_id.is_some() || state.frame_anchor_locked.is_some() {
            let mut unresolved = Vec::new();
            if state.paragraph_group_id.is_some() {
                unresolved.push("paragraph_group_border_margin_hierarchy_unresolved");
            }
            if state.frame_anchor_locked.is_some() {
                unresolved.push("frame_position_and_grouping_unresolved");
            }
            out.paragraph_contexts.push(ParagraphContext {
                paragraph_index: index,
                paragraph_group_id: state.paragraph_group_id,
                frame_anchor_locked: state.frame_anchor_locked,
                unresolved,
            });
        }
        if state.depth == 0 {
            flush(&mut stack, index, &mut out, "table_row_unterminated");
            out.outside_paragraphs.push(index);
            root_start_known = true;
            continue;
        }
        if state.depth > MAX_DEPTH {
            return Err(Error::Limit("table nesting depth"));
        }
        if state.depth < stack.len() {
            flush(
                &mut stack,
                index,
                &mut out,
                "table_depth_transition_unresolved",
            );
            root_start_known = false;
        }
        if stack.is_empty() && !root_start_known {
            out.unresolved_ranges.push(UnresolvedRange {
                paragraph_range: [index, index + 1],
                reason: "table_parent_cell_unresolved",
            });
            root_start_known = state.depth == 1 && state.row;
            continue;
        }
        // A cell may begin with a nested table before its own final paragraph.
        // Pending ancestors acquire ownership only when their cell and row marks close.
        while state.depth > stack.len() {
            stack.push(Frame {
                start: index,
                cell_start: index,
                cells: Vec::new(),
                content: Vec::new(),
                revision: false,
            });
        }
        if let Some(frame) = stack.last_mut() {
            frame.revision |= state.revision;
        }
        if state.row {
            let frame = stack.pop().ok_or(Error::Invalid("table row stack"))?;
            if !frame.content.is_empty()
                || frame.cells.is_empty()
                || paragraphs[index].source_range[1] - paragraphs[index].source_range[0] != 1
            {
                out.unresolved_ranges.push(UnresolvedRange {
                    paragraph_range: [frame.start, index + 1],
                    reason: "table_row_terminator_invalid",
                });
                flush(&mut stack, index + 1, &mut out, "nested_row_incomplete");
                root_start_known = false;
                continue;
            }
            let mut cells = frame.cells;
            let mut unresolved = Vec::new();
            let count = state.definition_known.then_some(state.definitions.len());
            if count != Some(cells.len()) {
                unresolved.push("row_cell_definition_count_unresolved");
            } else {
                for (cell, definition) in cells.iter_mut().zip(state.definitions) {
                    if definition.horizontal_merge != 0 || definition.vertical_merge != 0 {
                        unresolved.push("merged_cell_visible_ownership_unresolved");
                    }
                    cell.definition = Some(definition);
                }
            }
            if frame.revision
                || paragraphs[frame.start..=index].iter().any(|p| {
                    p.unresolved
                        .contains(&"suppressed_terminal_visible_topology_unresolved")
                })
            {
                unresolved.push("revised_or_hidden_row_topology_unresolved");
            }
            unresolved.sort_unstable();
            unresolved.dedup();
            let (source_range, projected_bytes) = range(paragraphs, frame.start, index + 1);
            let row_index = out.rows.len();
            out.rows.push(OwnedRow {
                depth: state.depth,
                source_range,
                projected_bytes,
                row_mark_paragraph: index,
                cells,
                definition_cell_count: count,
                unresolved,
            });
            if let Some(parent) = stack.last_mut() {
                parent.revision |= out.rows[row_index]
                    .unresolved
                    .contains(&"revised_or_hidden_row_topology_unresolved");
                parent.content.push(CellContent::NestedRow(row_index));
            }
        } else {
            let frame = stack.last_mut().ok_or(Error::Invalid("table cell stack"))?;
            frame.content.push(CellContent::Paragraph(index));
            if state.cell {
                if frame.cells.len() == 63 {
                    return Err(Error::Invalid("table row exceeds cell limit"));
                }
                let (source_range, projected_bytes) =
                    range(paragraphs, frame.cell_start, index + 1);
                frame.cells.push(OwnedCell {
                    source_range,
                    projected_bytes,
                    content: std::mem::take(&mut frame.content),
                    definition: None,
                });
                frame.cell_start = index + 1;
            }
        }
    }
    flush(
        &mut stack,
        paragraphs.len(),
        &mut out,
        "table_row_unterminated",
    );
    verify_ownership(&mut out, paragraphs.len())?;
    Ok(out)
}

/// Verify direct ownership independently of enclosing range overlap, then retain all unowned gaps.
/// Completed nested rows survive an interrupted ancestor and report their missing parent explicitly.
fn verify_ownership(out: &mut TableStory, count: usize) -> Result<()> {
    let mut owned = vec![false; count];
    let mut parents = vec![0usize; out.rows.len()];
    let mut mark = |index: usize| -> Result<()> {
        let slot = owned
            .get_mut(index)
            .ok_or(Error::Invalid("table paragraph ownership index"))?;
        if *slot {
            return Err(Error::Invalid("duplicate table paragraph ownership"));
        }
        *slot = true;
        Ok(())
    };
    for (index, row) in out.rows.iter().enumerate() {
        mark(row.row_mark_paragraph)?;
        for cell in &row.cells {
            for content in &cell.content {
                match *content {
                    CellContent::Paragraph(p) => mark(p)?,
                    CellContent::NestedRow(child) => {
                        if child >= index || out.rows[child].depth != row.depth + 1 {
                            return Err(Error::Invalid("table nested row ownership"));
                        }
                        parents[child] += 1;
                        if parents[child] > 1 {
                            return Err(Error::Invalid("duplicate nested row ownership"));
                        }
                    }
                }
            }
        }
    }
    for &index in &out.outside_paragraphs {
        mark(index)?;
    }
    for (row, parents) in out.rows.iter_mut().zip(parents) {
        if row.depth > 1 && parents == 0 {
            row.unresolved.push("enclosing_cell_ownership_unresolved");
        }
    }
    let mut reasons = vec![None; count];
    let mut work = MAX_ITEMS * 4;
    for range in &out.unresolved_ranges {
        if range.paragraph_range[0] > range.paragraph_range[1] || range.paragraph_range[1] > count {
            return Err(Error::Invalid("table unresolved paragraph range"));
        }
        work = work
            .checked_sub(range.paragraph_range[1] - range.paragraph_range[0])
            .ok_or(Error::Limit("table unresolved ownership work"))?;
        for reason in &mut reasons[range.paragraph_range[0]..range.paragraph_range[1]] {
            *reason = Some(range.reason);
        }
    }
    out.unresolved_ranges.clear();
    let mut index = 0;
    while index < count {
        if owned[index] {
            index += 1;
            continue;
        }
        let start = index;
        let reason = reasons[index].unwrap_or("table_ownership_unresolved");
        index += 1;
        while index < count
            && !owned[index]
            && reasons[index].unwrap_or("table_ownership_unresolved") == reason
        {
            index += 1;
        }
        out.unresolved_ranges.push(UnresolvedRange {
            paragraph_range: [start, index],
            reason,
        });
    }
    Ok(())
}

/// Direct paragraph membership is independent from border/margin groups and frame locking.
/// PIpgp group hierarchy and frame geometry require separate validation before table grouping.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/484822ee-a9d9-4af4-8423-29fda67a6a58>
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/ca1d2cfd-1639-4c52-a365-9dbc212e476b>
fn resolve(
    data: &[u8],
    prm: u16,
    groups: &[Vec<u8>],
    terminal: Option<u16>,
    version: u16,
    work: &mut usize,
) -> Result<State> {
    if !matches!(terminal, Some(7 | 12 | 13)) {
        return Err(Error::Unsupported("table paragraph terminal control"));
    }
    let mut ops = operators(data, false, work)?;
    if prm & 1 != 0 {
        ops.extend(operators(
            groups
                .get((prm >> 1) as usize)
                .ok_or(Error::Invalid("table PRM group"))?,
            true,
            work,
        )?);
    } else if prm != 0 {
        match (prm >> 1) & 0x7f {
            0x18 => ops.push((0x2416, vec![(prm >> 8) as u8])),
            0x19 => ops.push((0x2417, vec![(prm >> 8) as u8])),
            _ => {
                crate::office::doc::visibility::piece_modifiers(prm, groups)?;
            }
        }
    }
    let mut in_table = false;
    let mut depth = 0i32;
    let mut explicit = false;
    let mut outer_row = false;
    let mut inner_row = false;
    let mut inner_cell = false;
    let mut state = State {
        definition_known: true,
        ..State::default()
    };
    for (code, b) in ops {
        match code {
            // PGPInfo supplies borders/margins; PFLocked controls frame anchoring.
            // Their grouping context is retained separately from direct cell membership.
            0x6465 => state.paragraph_group_id = Some(u32_at(&b, 0)?),
            0x2430 => {
                state.frame_anchor_locked = Some(match b.as_slice() {
                    [0] => false,
                    [1] => true,
                    _ => return Err(Error::Unsupported("invalid table frame lock Boolean")),
                });
            }
            0x2416 | 0x2417 | 0x244b | 0x244c => {
                let value = match b.as_slice() {
                    [0] => false,
                    [1] => true,
                    _ => return Err(Error::Invalid("table paragraph Boolean")),
                };
                match code {
                    0x2416 => in_table = value,
                    0x2417 => outer_row = value,
                    0x244b => inner_cell = value,
                    _ => inner_row = value,
                }
            }
            0x6649 | 0x664a => {
                let value = u32_at(&b, 0)? as i32;
                depth = if code == 0x6649 {
                    value
                } else {
                    depth
                        .checked_add(value)
                        .ok_or(Error::Invalid("table depth overflow"))?
                };
                explicit = true;
                if depth < 0 {
                    return Err(Error::Invalid("table negative depth"));
                }
            }
            0xd608 => state.definitions = decode_table_definition(&b)?.cells,
            0x7621 => {
                let at = b[0] as usize;
                let count = b[1] as usize;
                let width = i16::from_le_bytes([b[2], b[3]]);
                if count == 0
                    || at > state.definitions.len()
                    || state.definitions.len() + count > 63
                    || width < 0
                {
                    return Err(Error::Invalid("table insertion"));
                }
                state.definitions.splice(
                    at..at,
                    (0..count).map(|_| CellDefinition {
                        horizontal_merge: 0,
                        vertical_merge: 0,
                        raw_flags: 0,
                        preferred_width_raw: width,
                        preferred_width_unit: 3,
                        definition_source: "TInsert_XAS_twips",
                    }),
                );
            }
            0x5622 | 0x5624 | 0x5625 => {
                let a = b[0] as usize;
                let z = b[1] as usize;
                if a >= z || z > state.definitions.len() {
                    return Err(Error::Invalid("table cell definition range"));
                }
                if code == 0x5622 {
                    state.definitions.drain(a..z);
                    if state.definitions.is_empty() {
                        return Err(Error::Invalid("table deletes all definitions"));
                    }
                } else {
                    for (i, c) in state.definitions[a..z].iter_mut().enumerate() {
                        c.horizontal_merge = if code == 0x5625 {
                            0
                        } else if i == 0 {
                            2
                        } else {
                            1
                        };
                    }
                }
            }
            0xd62b => {
                if b.len() != 3 || b[0] != 2 || !matches!(b[2], 0 | 1 | 3) {
                    return Err(Error::Invalid("table vertical merge operand"));
                }
                let cell = state
                    .definitions
                    .get_mut(b[1] as usize)
                    .ok_or(Error::Invalid("table vertical merge cell"))?;
                cell.vertical_merge = b[2];
            }
            0x3668 | 0x2664 => {
                if b.len() != 1 || b[0] > 1 {
                    return Err(Error::Invalid("table revision wall operand"));
                }
                state.revision = true;
            }
            0xd667 | 0xc66f => {
                if b.len() != 8 || b[0] != 7 {
                    return Err(Error::Invalid("table property revision operand"));
                }
                state.revision = true;
            }
            code if (code >> 10) & 7 == 1 => {
                if !paragraph_structure_preserving(code) {
                    return Err(Error::Unsupported("table paragraph modifier"));
                }
            }
            code if (code >> 10) & 7 == 5 && !table_structure_preserving(code) => {
                state.definition_known = false;
            }
            _ => {}
        }
    }
    if version == 0xc1 && in_table && !explicit {
        depth = 1;
    }
    if in_table != (depth > 0) {
        return Err(Error::Unsupported("table depth membership disagreement"));
    }
    state.depth = depth as usize;
    match depth {
        0 => {
            if outer_row || inner_row || inner_cell || terminal == Some(7) {
                return Err(Error::Unsupported("table zero-depth marker"));
            }
        }
        1 => {
            if inner_row || inner_cell || outer_row && terminal != Some(7) {
                return Err(Error::Unsupported("table outer marker"));
            }
            state.row = outer_row;
            state.cell = terminal == Some(7) && !outer_row;
        }
        _ => {
            if outer_row || (inner_row || inner_cell) && terminal != Some(13) {
                return Err(Error::Unsupported("table inner marker"));
            }
            state.row = inner_row;
            // Inner TTP refines the cell-mark designation, as PFTtp does at depth one.
            state.cell = inner_cell && !inner_row;
        }
    }
    Ok(state)
}
fn operators(data: &[u8], paragraph_only: bool, work: &mut usize) -> Result<Vec<(u16, Vec<u8>)>> {
    *work = work
        .checked_sub(data.len() + 1)
        .ok_or(Error::Limit("table modifier work"))?;
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        let code = u16_at(data, i)?;
        if !(1..=5).contains(&((code >> 10) & 7)) {
            return Err(Error::Unsupported("table property scope"));
        }

        i += 2;
        let n = crate::office::doc::paragraph_references::operand_length(code, &data[i..])?;
        let b = bytes(data, i, n)?;
        i += n;
        if !paragraph_only || (code >> 10) & 7 == 1 {
            out.push((code, b.to_vec()));
        }
    }
    Ok(out)
}
fn paragraph_structure_preserving(code: u16) -> bool {
    matches!(
        code,
        0x2441
            | 0xc645
            | 0x2443
            | 0x443a
            | 0x4600
            | 0xc601
            | 0x2602
            | 0x2403
            | 0x2405
            | 0x2406
            | 0x2407
            | 0x240c
            | 0xc60d
            | 0x840e
            | 0x840f
            | 0x4610
            | 0x8411
            | 0x6412
            | 0xa413
            | 0xa414
            | 0xc615
            | 0x460b
            | 0x260a
            | 0x242a
            | 0x2431
            | 0x2433
            | 0x2434
            | 0x2435
            | 0x2436
            | 0x2447
            | 0x2462
            | 0x246d
            | 0x8418
            | 0x8419
            | 0x841a
            | 0x261b
            | 0x2423
            | 0x442b
            | 0x442c
            | 0x442d
            | 0x842e
            | 0x842f
            | 0xc64d
            | 0xc64e
            | 0xc64f
            | 0xc650
            | 0xc651
            | 0xc652
            | 0xc653
            | 0x6629
            | 0xc66c
            | 0x4455
            | 0x4456
            | 0x4457
            | 0x4458
            | 0x4459
            | 0x4439
            | 0x6424
            | 0x6425
            | 0x6426
            | 0x6427
            | 0x6428
            | 0x6429
            | 0x2437
            | 0x2438
            | 0x2448
            | 0x245b
            | 0x245c
            | 0x845d
            | 0x845e
            | 0x8460
            | 0x2461
            | 0x6467
            | 0x2640
    )
}
fn table_structure_preserving(code: u16) -> bool {
    matches!(
        code,
        0x548a
            | 0x360d
            | 0x5400
            | 0x9601
            | 0x9602
            | 0x3403
            | 0x3404
            | 0xd605
            | 0x9407
            | 0xd609
            | 0x740a
            | 0x560b
            | 0x740c
            | 0x940e
            | 0x940f
            | 0x9410
            | 0x9411
            | 0xd612
            | 0xd613
            | 0xf614
            | 0x3615
            | 0xd616
            | 0xf617
            | 0xf618
            | 0x3619
            | 0xd61a
            | 0xd61b
            | 0xd61c
            | 0xd61d
            | 0x941e
            | 0x941f
            | 0xd620
            | 0x7623
            | 0x7629
            | 0xd62c
            | 0xd62d
            | 0xd62e
            | 0xd62f
            | 0xd632
            | 0xd633
            | 0xd634
            | 0xd635
            | 0xf636
            | 0xd639
            | 0x563a
            | 0xd63e
            | 0xd642
            | 0xd660
            | 0xf661
            | 0xd662
            | 0x5664
            | 0x3465
            | 0x3466
            | 0x7469
            | 0xd670
            | 0xd671
            | 0xd672
            | 0x7479
    )
}

#[cfg(test)]
mod tests;
