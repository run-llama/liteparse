//! Physical nested-table membership used to isolate unresolved list families.
use crate::office::doc::{
    Error, Result,
    structure::StoryStructure,
    table_structure::{CellContent, TableStory},
    visibility::RevisionEvidence,
};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Paragraph {
    pub root: [usize; 2],
    pub depth: u32,
    pub cell_end: bool,
    pub row_end: bool,
    pub columns: u8,
}

// MS-DOC 2.4.3: nested cells and rows end in distinct flagged CR paragraphs.
// A complete enclosing row gives a boundary at which outside counters can resume.
// This proves physical membership only; all affected list families remain unresolved.
pub(super) fn collect(
    part: &StoryStructure,
    table: &TableStory,
    revisions: &RevisionEvidence,
    work: &mut usize,
) -> Result<BTreeMap<[usize; 2], Paragraph>> {
    let mut out = BTreeMap::new();
    for (root_index, root) in table
        .rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.depth == 1)
    {
        if !root
            .cells
            .iter()
            .flat_map(|cell| &cell.content)
            .any(|c| matches!(c, CellContent::NestedRow(_)))
        {
            continue;
        }
        let mut revision_end = root.source_range[0];
        for span in revisions.spans.iter().skip(
            revisions
                .spans
                .partition_point(|s| s.end_cp <= revision_end),
        ) {
            if span.start_cp >= root.source_range[1] {
                break;
            }
            super::charge(work, 1)?;
            if span.start_cp > revision_end
                || span.properties.deleted != Some(false)
                || span.properties.inserted != Some(false)
                || !span.unresolved.is_empty()
            {
                break;
            }
            revision_end = span.end_cp.min(root.source_range[1]);
        }
        if revision_end != root.source_range[1] {
            continue;
        }
        let mut pending = vec![(root_index, 1)];
        let mut records = BTreeMap::new();
        let mut valid = true;
        while let Some((index, depth)) = pending.pop() {
            super::charge(work, 1)?;
            let row = table
                .rows
                .get(index)
                .ok_or(Error::Invalid("nested list row index"))?;
            if depth > 64
                || row.depth != depth
                || !row.unresolved.is_empty()
                || row.definition_cell_count != Some(row.cells.len())
                || row.cells.is_empty()
                || row.cells.len() > 63
                || row.source_range[0] < root.source_range[0]
                || row.source_range[1] > root.source_range[1]
            {
                valid = false;
                break;
            }
            let columns = row.cells.len() as u8;
            for cell in &row.cells {
                for (i, content) in cell.content.iter().enumerate() {
                    super::charge(work, 1)?;
                    match *content {
                        CellContent::NestedRow(child) => {
                            if child >= index {
                                return Err(Error::Invalid("nested list row order"));
                            }
                            pending.push((child, depth + 1));
                        }
                        CellContent::Paragraph(p) => {
                            if records
                                .insert(
                                    p,
                                    Paragraph {
                                        root: root.source_range,
                                        depth: depth as u32,
                                        cell_end: i + 1 == cell.content.len(),
                                        row_end: false,
                                        columns,
                                    },
                                )
                                .is_some()
                            {
                                return Err(Error::Invalid("nested list duplicate paragraph"));
                            }
                        }
                    }
                }
            }
            if records
                .insert(
                    row.row_mark_paragraph,
                    Paragraph {
                        root: root.source_range,
                        depth: depth as u32,
                        cell_end: false,
                        row_end: true,
                        columns,
                    },
                )
                .is_some()
            {
                return Err(Error::Invalid("nested list duplicate row mark"));
            }
        }
        if !valid {
            continue;
        }
        let mut cp = root.source_range[0];
        for (&index, record) in &records {
            let p = part
                .paragraphs
                .get(index)
                .ok_or(Error::Invalid("nested list paragraph index"))?;
            let terminal = if record.depth == 1 && (record.cell_end || record.row_end) {
                7
            } else {
                13
            };
            if p.source_range[0] != cp
                || p.source_range[1] <= cp
                || p.terminal_unit != Some(terminal)
                || !p.unresolved.is_empty()
                || (record.row_end && p.source_range[1] != cp + 1)
            {
                valid = false;
                break;
            }
            cp = p.source_range[1];
        }
        if !valid || cp != root.source_range[1] {
            continue;
        }
        for (index, record) in records {
            if out
                .insert(part.paragraphs[index].source_range, record)
                .is_some()
            {
                return Err(Error::Invalid("nested list overlapping roots"));
            }
        }
    }
    Ok(out)
}

/// Bind direct row members to flagged physical paragraphs, excluding child-row content.
pub(super) fn direct_members(
    table: &TableStory,
    row_index: usize,
    part: &StoryStructure,
    records: &BTreeMap<[usize; 2], Paragraph>,
    paragraphs: &crate::office::doc::paragraph_formatting::ParagraphInspection,
    formatting: &crate::office::doc::formatting_inspection::FormattingInspection,
    work: &mut usize,
) -> Result<Option<std::collections::BTreeSet<usize>>> {
    let row = table
        .rows
        .get(row_index)
        .ok_or(Error::Invalid("nested baseline row index"))?;
    let mut direct = std::collections::BTreeSet::new();
    for cell in &row.cells {
        for content in &cell.content {
            if let CellContent::Paragraph(index) = content
                && !direct.insert(*index)
            {
                return Err(Error::Invalid("nested baseline duplicate member"));
            }
        }
    }
    if !direct.insert(row.row_mark_paragraph) {
        return Err(Error::Invalid("nested baseline row mark"));
    }
    for index in &direct {
        super::charge(work, 32)?;
        let physical = part
            .paragraphs
            .get(*index)
            .ok_or(Error::Invalid("nested baseline member"))?;
        let Some(record) = records.get(&physical.source_range) else {
            return Ok(None);
        };
        if record.depth as usize != row.depth
            || record.row_end != (*index == row.row_mark_paragraph)
        {
            return Ok(None);
        }
        let end = physical.source_range[1];
        let pap = paragraphs
            .spans
            .get(paragraphs.spans.partition_point(|p| p.end_cp < end))
            .filter(|p| [p.start_cp, p.end_cp] == physical.source_range)
            .ok_or(Error::Invalid("nested baseline PAPX"))?;
        let piece = formatting
            .pieces
            .get(formatting.pieces.partition_point(|p| p.end_cp < end))
            .filter(|p| p.start_cp < end && p.end_cp >= end)
            .ok_or(Error::Invalid("nested baseline piece"))?;
        let mut properties = super::paragraph::group(&pap.raw_modifiers, 1, work)?;
        properties.extend(super::paragraph::piece(
            piece.raw_prm,
            &formatting.raw_property_groups,
            1,
            work,
        )?);
        let (mut inside, mut outer, mut inner_cell, mut inner_row, mut depth) =
            (false, false, false, false, 0i32);
        for p in properties {
            let flag = |value: &[u8]| match value {
                [0] => Ok(false),
                [1] => Ok(true),
                _ => Err(Error::Invalid("nested baseline Boolean")),
            };
            match p.sprm {
                0x2416 => inside = flag(&p.operand)?,
                0x2417 => outer = flag(&p.operand)?,
                0x244b => inner_cell = flag(&p.operand)?,
                0x244c => inner_row = flag(&p.operand)?,
                0x6649 => depth = crate::office::doc::u32_at(&p.operand, 0)? as i32,
                0x664a => {
                    depth = depth
                        .checked_add(crate::office::doc::u32_at(&p.operand, 0)? as i32)
                        .ok_or(Error::Invalid("nested baseline depth"))?
                }
                _ => {}
            }
        }
        if !inside
            || depth != row.depth as i32
            || outer != (row.depth == 1 && record.row_end)
            || (row.depth > 1
                && (inner_row != record.row_end
                    || inner_cell != (record.cell_end || record.row_end)))
        {
            return Ok(None);
        }
    }
    Ok(Some(direct))
}
