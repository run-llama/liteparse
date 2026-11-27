//! Nested tables occupy exact child intervals inside one physical parent cell.
use super::*;
use std::collections::BTreeSet;

/// Enumerate source tables with bounded depth before any mutable recursive walk.
pub fn tables(tables: &[SourceTable]) -> Result<Vec<&SourceTable>> {
    let mut out = Vec::new();
    let mut stack: Vec<_> = tables.iter().rev().map(|t| (t, 1usize)).collect();
    let mut work = 0usize;
    while let Some((table, depth)) = stack.pop() {
        if depth > 8 || out.len() >= 16384 {
            return Err(Error::Limit("nested table traversal"));
        }
        out.push(table);
        for row in table.rows.iter().rev() {
            for cell in row.cells.iter().rev() {
                work += 1;
                if work > 65536 || stack.len() + cell.nested_tables.len() > 16384 {
                    return Err(Error::Limit("nested cell traversal"));
                }
                stack.extend(cell.nested_tables.iter().rev().map(|t| (t, depth + 1)));
            }
        }
    }
    Ok(out)
}

pub fn visit_paragraphs_mut(
    source: &mut [SourceTable],
    visit: &mut impl FnMut(&mut CellParagraph) -> Result<()>,
) -> Result<()> {
    tables(source)?;
    visit_bounded(source, visit)
}

fn visit_bounded(
    tables: &mut [SourceTable],
    visit: &mut impl FnMut(&mut CellParagraph) -> Result<()>,
) -> Result<()> {
    for table in tables {
        for row in &mut table.rows {
            for cell in &mut row.cells {
                for paragraph in &mut cell.paragraphs {
                    visit(paragraph)?;
                }
                visit_bounded(&mut cell.nested_tables, visit)?;
            }
        }
    }
    Ok(())
}

struct Ledger<'a> {
    paragraphs: &'a [crate::office::doc::render_model::plain::PlainParagraph],
    version: u16,
    marks: BTreeSet<usize>,
    seen: BTreeSet<usize>,
    budget: usize,
    rows: usize,
}

pub(super) fn validate(
    tables: &[SourceTable],
    paragraphs: &[crate::office::doc::render_model::plain::PlainParagraph],
    version: u16,
) -> Result<BTreeSet<usize>> {
    let mut ledger = Ledger {
        paragraphs,
        version,
        marks: BTreeSet::new(),
        seen: BTreeSet::new(),
        budget: 8 * 1024 * 1024,
        rows: 0,
    };
    ledger.tables(tables, 1, None)?;
    Ok(ledger.marks)
}

impl Ledger<'_> {
    fn charge_membership(&mut self, properties: &[PropertyModifier]) -> Result<()> {
        for p in properties {
            self.budget = self
                .budget
                .checked_sub(p.operand.len() + std::mem::size_of::<PropertyModifier>())
                .ok_or(Error::Limit("nested membership work"))?;
        }
        Ok(())
    }
    fn paragraph(&mut self, member: &CellParagraph, cp: usize) -> Result<usize> {
        let p = self
            .paragraphs
            .get(member.physical_index)
            .ok_or(Error::Invalid("render cell index"))?;
        if !self.seen.insert(member.physical_index)
            || p.source_cp != member.source_cp
            || p.source_cp[0] != cp
            || p.source_cp[1] <= cp
            || p.source_style != member.source_style
            || p.properties != member.properties
        {
            return Err(Error::Invalid("render cell paragraph ledger"));
        }
        Ok(p.source_cp[1])
    }
    fn tables(
        &mut self,
        tables: &[SourceTable],
        depth: usize,
        parent: Option<[usize; 2]>,
    ) -> Result<()> {
        if depth > 8 || tables.len() > 16384 {
            return Err(Error::Limit("nested table model"));
        }
        let mut previous = parent.map_or(0, |r| r[0]);
        for table in tables {
            if table.source_cp[0] < previous
                || table.source_cp[0] >= table.source_cp[1]
                || parent.is_some_and(|r| table.source_cp[1] > r[1])
                || table.rows.is_empty()
            {
                return Err(Error::Invalid("render table model order"));
            }
            let mut cp = table.source_cp[0];
            let mut layouts = Vec::new();
            for row in &table.rows {
                self.rows += 1;
                if self.rows > 16384 {
                    return Err(Error::Limit("nested table rows"));
                }
                if row.depth != depth
                    || row.source_cp[0] != cp
                    || row.cells.len() != row.layout.cells.len()
                {
                    return Err(Error::Invalid("render table row ledger"));
                }
                validate_membership(&row.table_membership, depth, true, true)?;
                self.charge_membership(&row.table_membership)?;
                charge_row_evidence(&row.layout, &mut self.budget)?;
                if super::super::decode_recorded_row(self.version, &row.layout)? != row.layout {
                    return Err(Error::Invalid("render table layout provenance"));
                }
                for cell in &row.cells {
                    if cell.source_cp[0] != cp || cell.paragraphs.is_empty() {
                        return Err(Error::Invalid("render table cell ledger"));
                    }
                    let mut child = 0;
                    for member in &cell.paragraphs {
                        while cell
                            .nested_tables
                            .get(child)
                            .is_some_and(|t| t.source_cp[0] < member.source_cp[0])
                        {
                            let table = &cell.nested_tables[child];
                            if table.source_cp[0] != cp || table.source_cp[1] > member.source_cp[0]
                            {
                                return Err(Error::Invalid("nested table cell interval"));
                            }
                            self.tables(
                                std::slice::from_ref(table),
                                depth + 1,
                                Some(cell.source_cp),
                            )?;
                            cp = table.source_cp[1];
                            child += 1;
                        }
                        validate_membership(
                            &member.table_membership,
                            depth,
                            false,
                            member.source_cp[1] == cell.source_cp[1],
                        )?;
                        self.charge_membership(&member.table_membership)?;
                        cp = self.paragraph(member, cp)?;
                    }
                    if child != cell.nested_tables.len() || cp != cell.source_cp[1] {
                        return Err(Error::Invalid("render cell end ledger"));
                    }
                    if depth == 1 {
                        self.marks.insert(cp - 1);
                    }
                }
                let p = self
                    .paragraphs
                    .get(row.row_mark_paragraph)
                    .ok_or(Error::Invalid("render row mark index"))?;
                if !self.seen.insert(row.row_mark_paragraph)
                    || p.source_cp != [cp, cp + 1]
                    || p.source_style != row.row_mark_style
                    || p.properties != row.row_mark_properties
                    || row.source_cp[1] != cp + 1
                    || (depth > 1
                        && p.runs.iter().map(|r| r.text.as_str()).collect::<String>() != "\r")
                {
                    return Err(Error::Invalid("render row mark ledger"));
                }
                if depth == 1 {
                    self.marks.insert(cp);
                }
                cp += 1;
                layouts.push(row.layout.clone());
            }
            super::super::merges::vertical_owners(&layouts)?;
            if cp != table.source_cp[1] || super::super::common_grid(&layouts)? != table.grid {
                return Err(Error::Invalid("render table grid ledger"));
            }
            previous = cp;
        }
        Ok(())
    }
}

pub(super) fn record_membership(
    source: &ParagraphInspection,
    range: [usize; 2],
    depth: usize,
) -> Result<Vec<PropertyModifier>> {
    if depth == 1 {
        return Ok(Vec::new());
    }
    let span = source
        .spans
        .get(source.spans.partition_point(|p| p.end_cp <= range[0]))
        .filter(|p| [p.start_cp, p.end_cp] == range)
        .ok_or(Error::Invalid("nested paragraph membership span"))?;
    Ok(decode(&span.raw_modifiers)?
        .into_iter()
        .filter(|p| matches!(p.sprm, 0x2416 | 0x2417 | 0x6649 | 0x244b | 0x244c))
        .collect())
}

pub(super) fn validate_membership(
    properties: &[PropertyModifier],
    depth: usize,
    row: bool,
    cell_end: bool,
) -> Result<()> {
    if depth == 1 {
        return if properties.is_empty() {
            Ok(())
        } else {
            Err(Error::Invalid("unexpected depth-one membership extension"))
        };
    }
    let (mut in_table, mut outer_row, mut inner_row, mut inner_cell) = (false, false, false, false);
    let mut recorded = None;
    for p in properties {
        match p.sprm {
            0x2416 => in_table = super::super::bool8(&p.operand)?,
            0x2417 => outer_row = super::super::bool8(&p.operand)?,
            0x244b => inner_cell = super::super::bool8(&p.operand)?,
            0x244c => inner_row = super::super::bool8(&p.operand)?,
            0x6649 if p.operand.len() == 4 => {
                recorded = Some(crate::office::doc::u32_at(&p.operand, 0)? as usize)
            }
            _ => return Err(Error::Invalid("nested table membership operand")),
        }
    }
    if !in_table
        || outer_row
        || recorded != Some(depth)
        || inner_row != row
        || inner_cell != cell_end
    {
        return Err(Error::Invalid("nested table membership evidence"));
    }
    Ok(())
}
