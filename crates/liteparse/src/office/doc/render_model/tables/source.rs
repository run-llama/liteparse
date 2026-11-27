//! Bind row geometry to physical source paragraphs without inferring table groups.
use super::RowLayout;
use crate::office::doc::{
    Error, Inspection, Result, formatting::PropertyModifier,
    formatting_inspection::FormattingInspection, paragraph_formatting::ParagraphInspection,
    render_model::paragraphs, structure::StructurePlan, styles::Stylesheet,
    table_structure::TableOwnership,
};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct SourceRow {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub table_membership: Vec<PropertyModifier>,
    #[serde(skip_serializing_if = "depth_one")]
    pub depth: usize,
    pub source_cp: [usize; 2],
    pub row_mark_paragraph: usize,
    pub row_mark_style: usize,
    pub row_mark_properties: paragraphs::ParagraphProperties,
    pub cells: Vec<SourceCell>,
    pub layout: RowLayout,
}
#[derive(Debug, Serialize)]
pub struct SourceCell {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub nested_tables: Vec<SourceTable>,
    pub source_cp: [usize; 2],
    pub paragraphs: Vec<CellParagraph>,
}
#[derive(Debug, Serialize)]
pub struct CellParagraph {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub table_membership: Vec<PropertyModifier>,
    pub physical_index: usize,
    pub source_cp: [usize; 2],
    pub source_style: usize,
    pub properties: paragraphs::ParagraphProperties,
}

#[derive(Debug, Serialize)]
pub struct SourceTable {
    pub source_cp: [usize; 2],
    pub rows: Vec<SourceRow>,
    pub grid: super::GridPlan,
    pub grouping_basis: &'static str,
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde skip_serializing_if passes a reference"
)]
fn depth_one(depth: &usize) -> bool {
    *depth == 1
}
pub mod nested;

type PendingLists = std::collections::BTreeMap<[usize; 2], Vec<PropertyModifier>>;
type BoundTables = (Vec<SourceTable>, PendingLists);

struct TableInput<'a> {
    word: &'a [u8],
    pending: Option<&'a mut PendingLists>,
}

// Public models can repeat identical style chains on many rows. Charge each replay
// against one document budget, including empty records and ancestry traversal.
fn charge_row_evidence(row: &RowLayout, budget: &mut usize) -> Result<()> {
    let mut charge = |size: usize| -> Result<()> {
        *budget = budget
            .checked_sub(size)
            .ok_or(Error::Limit("render table ledger work"))?;
        Ok(())
    };
    charge(std::mem::size_of::<RowLayout>())?;
    for property in &row.applied {
        charge(std::mem::size_of::<PropertyModifier>())?;
        charge(property.operand.len())?;
    }
    for style in &row.style_defaults {
        charge(std::mem::size_of::<super::style_defaults::CellDefaults>())?;
        for _ in &style.base_to_derived {
            charge(std::mem::size_of::<usize>())?;
        }
        for (_, property) in &style.applied {
            charge(std::mem::size_of::<(usize, PropertyModifier)>())?;
            charge(property.operand.len())?;
        }
    }
    Ok(())
}

// Defer only direct list selectors until the complete source family is resolved.
// Other paragraph properties and style-origin selectors keep their normal guards.
fn resolve_cell_paragraph(
    styles: &Stylesheet,
    style: usize,
    direct: &[PropertyModifier],
    range: [usize; 2],
    pending: Option<&mut std::collections::BTreeMap<[usize; 2], Vec<PropertyModifier>>>,
) -> Result<paragraphs::ParagraphProperties> {
    let Some(pending) =
        pending.filter(|_| direct.iter().any(|p| matches!(p.sprm, 0x260a | 0x460b)))
    else {
        return paragraphs::resolve(styles, style, direct);
    };
    for property in direct {
        if property.sprm == 0x260a && (property.operand.len() != 1 || property.operand[0] > 8)
            || property.sprm == 0x460b && property.operand.len() != 2
        {
            return Err(Error::Invalid("table list selector operand"));
        }
    }
    let ordinary: Vec<_> = direct
        .iter()
        .filter(|p| !matches!(p.sprm, 0x260a | 0x460b))
        .cloned()
        .collect();
    let resolved = paragraphs::resolve(styles, style, &ordinary)?;
    if pending.contains_key(&range) {
        return Err(Error::Invalid("duplicate table list paragraph"));
    }
    pending.insert(range, direct.to_vec());
    Ok(resolved)
}

/// Validate the mutable public model's table-to-flat-paragraph ledger.
/// The returned positions are the only U0007 controls admitted as cell/row marks.
pub fn validate_ledger(
    tables: &[SourceTable],
    paragraphs: &[crate::office::doc::render_model::plain::PlainParagraph],
    version: u16,
) -> Result<std::collections::BTreeSet<usize>> {
    nested::validate(tables, paragraphs, version)
}

/// Group source-qualified adjacent rows under the default nonfloating table identity.
/// The closed property grammar excludes table identity overrides and paragraph frames;
/// protection bookmarks are checked separately because they can split adjacent rows.
/// <https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/5b45f0e7-7760-4fdb-af88-0146de2feb4c>
pub fn bind_tables(
    word: &[u8],
    source: &Inspection,
    structure: &StructurePlan,
    paragraphs: &ParagraphInspection,
    formatting: &FormattingInspection,
    tables: &TableOwnership,
    styles: &Stylesheet,
) -> Result<Vec<SourceTable>> {
    bind_tables_inner(
        TableInput {
            word,
            pending: None,
        },
        source,
        structure,
        paragraphs,
        formatting,
        tables,
        styles,
    )
}

pub(in crate::office::doc::render_model) fn bind_tables_deferred(
    word: &[u8],
    source: &Inspection,
    structure: &StructurePlan,
    paragraphs: &ParagraphInspection,
    formatting: &FormattingInspection,
    tables: &TableOwnership,
    styles: &Stylesheet,
) -> Result<BoundTables> {
    let mut pending = Default::default();
    let bound = bind_tables_inner(
        TableInput {
            word,
            pending: Some(&mut pending),
        },
        source,
        structure,
        paragraphs,
        formatting,
        tables,
        styles,
    )?;
    Ok((bound, pending))
}

fn bind_tables_inner(
    input: TableInput<'_>,
    source: &Inspection,
    structure: &StructurePlan,
    paragraphs: &ParagraphInspection,
    formatting: &FormattingInspection,
    tables: &TableOwnership,
    styles: &Stylesheet,
) -> Result<Vec<SourceTable>> {
    let TableInput { word, pending } = input;
    let (version, _, lengths, _, _) = crate::office::doc::document::fib(word)?;
    if version != source.binary_version
        || source
            .stories
            .first()
            .is_none_or(|s| s.utf16_units != lengths[0])
    {
        return Err(Error::Invalid("render table FIB identity"));
    }
    protection_absent(word)?;
    let rows = bind_rows_inner(
        source, structure, paragraphs, formatting, tables, styles, pending,
    )?;
    group_rows(rows)
}

fn group_rows(rows: Vec<SourceRow>) -> Result<Vec<SourceTable>> {
    let mut groups: Vec<Vec<SourceRow>> = Vec::new();
    for row in rows {
        if groups.last().and_then(|g| g.last()).is_some_and(|p| {
            p.source_cp[1] == row.source_cp[0]
                && p.depth == row.depth
                && selected_style(&p.layout) == selected_style(&row.layout)
        }) {
            groups.last_mut().unwrap().push(row);
        } else {
            groups.push(vec![row]);
        }
    }
    groups
        .into_iter()
        .map(|rows| {
            let source_cp = [rows[0].source_cp[0], rows.last().unwrap().source_cp[1]];
            let layouts: Vec<_> = rows.iter().map(|r| r.layout.clone()).collect();
            let grid = super::common_grid(&layouts)?;
            super::merges::vertical_owners(&layouts)?;
            let grouping_basis = if rows[0].depth > 1 {
                "adjacent_nested_rows_with_exact_parent_cell_ownership"
            } else if rows.iter().all(|r| r.layout.style_defaults.is_empty()) {
                "adjacent_depth_one_default_identity_no_frames_or_protection"
            } else {
                "adjacent_depth_one_selected_style_identity_no_frames_or_protection"
            };
            Ok(SourceTable {
                source_cp,
                rows,
                grid,
                grouping_basis,
            })
        })
        .collect()
}

fn selected_style(row: &RowLayout) -> usize {
    row.applied
        .iter()
        .rev()
        .find(|p| p.sprm == 0x563a)
        .and_then(|p| crate::office::doc::u16_at(&p.operand, 0).ok())
        .map_or(11, usize::from)
}

fn protection_absent(word: &[u8]) -> Result<()> {
    let count = crate::office::doc::u16_at(word, 152)? as usize;
    // FibRgFcLcb2003 extends 136 pairs. Its protection name/start/end PLCs
    // occupy pairs 141–143; any present payload needs explicit range semantics.
    // https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/f6b7d624-570c-4057-acfd-cba71d12f1a0
    for index in 141..=143 {
        if index < count && crate::office::doc::u32_at(word, 158 + 8 * index)? != 0 {
            return Err(Error::Unsupported("render protected table grouping"));
        }
    }
    Ok(())
}

fn decode(data: &[u8]) -> Result<Vec<PropertyModifier>> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < data.len() {
        if out.len() >= 262144 {
            return Err(Error::Limit("render row source modifiers"));
        }
        let sprm = crate::office::doc::u16_at(data, at)?;
        at += 2;
        let n = crate::office::doc::paragraph_references::operand_length(sprm, &data[at..])?;
        out.push(PropertyModifier {
            sprm,
            operand: crate::office::doc::bytes(data, at, n)?.to_vec(),
        });
        at += n;
    }
    Ok(out)
}

/// Resolve depth-one physical rows and retain every cell and row-mark source CP.
/// Selected table styles must have qualified text and cell contributions; nested
/// or merged rows and piece overrides remain unsupported. Text visibility and
/// actual run fonts remain separate checks.
pub fn bind_rows(
    source: &Inspection,
    structure: &StructurePlan,
    paragraphs: &ParagraphInspection,
    formatting: &FormattingInspection,
    tables: &TableOwnership,
    styles: &Stylesheet,
) -> Result<Vec<SourceRow>> {
    bind_rows_inner(
        source, structure, paragraphs, formatting, tables, styles, None,
    )
}

fn bind_rows_inner(
    source: &Inspection,
    structure: &StructurePlan,
    paragraphs: &ParagraphInspection,
    formatting: &FormattingInspection,
    tables: &TableOwnership,
    styles: &Stylesheet,
    mut pending: Option<&mut std::collections::BTreeMap<[usize; 2], Vec<PropertyModifier>>>,
) -> Result<Vec<SourceRow>> {
    if source.binary_version != paragraphs.binary_version
        || source.binary_version != formatting.binary_version
        || paragraphs.spans.len() > 262144
        || formatting.pieces.len() > 262144
        || source.stories.len() != structure.stories.len()
        || source.stories.len() != tables.stories.len()
    {
        return Err(Error::Invalid("render table source identity"));
    }
    if formatting.pieces.iter().any(|p| p.raw_prm != 0) {
        return Err(Error::Unsupported("render table style or piece properties"));
    }
    let body = source
        .stories
        .first()
        .ok_or(Error::Invalid("render table body"))?;
    let physical = structure
        .stories
        .first()
        .ok_or(Error::Invalid("render table structure"))?;
    let owned = tables
        .stories
        .first()
        .ok_or(Error::Invalid("render table ownership"))?;
    if body.kind != "body"
        || physical.kind != "body"
        || owned.kind != "body"
        || body.start_cp != 0
        || body.utf16_units > 1024 * 1024
        || physical.paragraphs.len() > 16384
        || owned.rows.len() > 16384
        || !owned.unresolved_ranges.is_empty()
    {
        return Err(Error::Unsupported("render table body ownership"));
    }
    let units: Vec<_> = body.text.encode_utf16().collect();
    if units.len() != body.utf16_units
        || paragraphs
            .spans
            .windows(2)
            .any(|w| w[0].end_cp != w[1].start_cp)
        || paragraphs.spans.iter().any(|p| p.start_cp >= p.end_cp)
    {
        return Err(Error::Invalid("render table source coverage"));
    }
    let mut next = 0;
    for p in &physical.paragraphs {
        if p.source_range[0] != next || p.source_range[1] <= next || p.source_range[1] > units.len()
        {
            return Err(Error::Invalid("render physical paragraph partition"));
        }
        next = p.source_range[1];
    }
    if next != units.len() {
        return Err(Error::Invalid("render physical paragraph end"));
    }
    let mut claimed = vec![false; physical.paragraphs.len()];
    let mut budget = 8 * 1024 * 1024usize;
    let mut style_budget = 8 * 1024 * 1024usize;
    let mut rows: Vec<Option<SourceRow>> = Vec::new();
    let mut previous_end = [0; 9];
    for (row_index, row) in owned.rows.iter().enumerate() {
        if !(1..=8).contains(&row.depth)
            || row
                .unresolved
                .iter()
                .any(|reason| *reason != "merged_cell_visible_ownership_unresolved")
            || row.cells.is_empty()
            || row.cells.len() > 63
            || row.source_range[0] < previous_end[row.depth]
        {
            return Err(Error::Unsupported("render row topology"));
        }
        let mark = physical
            .paragraphs
            .get(row.row_mark_paragraph)
            .ok_or(Error::Invalid("render row mark index"))?;
        if mark.source_range[1] != row.source_range[1]
            || mark.source_range[1] != mark.source_range[0] + 1
            || mark.terminal_unit != Some(if row.depth == 1 { 7 } else { 13 })
            || units[mark.source_range[0]] != if row.depth == 1 { 7 } else { 13 }
            || mark
                .endpoint
                .as_ref()
                .is_none_or(|e| e.end_cp != mark.source_range[1])
            || !mark.unresolved.is_empty()
        {
            return Err(Error::Unsupported("render row mark ownership"));
        }
        let mut cell_start = row.source_range[0];
        let mut cells = Vec::new();
        for cell in &row.cells {
            if cell.source_range[0] != cell_start
                || cell.source_range[1] <= cell_start
                || cell.source_range[1] > mark.source_range[0]
            {
                return Err(Error::Invalid("render cell source partition"));
            }
            let mut members = Vec::new();
            let mut children = Vec::new();
            for content in &cell.content {
                let index = match content {
                    crate::office::doc::table_structure::CellContent::Paragraph(index) => *index,
                    crate::office::doc::table_structure::CellContent::NestedRow(child) => {
                        if *child >= row_index {
                            return Err(Error::Invalid("nested row forward ownership"));
                        }
                        let child = rows
                            .get_mut(*child)
                            .and_then(Option::take)
                            .ok_or(Error::Invalid("nested row duplicate ownership"))?;
                        if child.depth != row.depth + 1
                            || child.source_cp[0] != cell_start
                            || child.source_cp[1] > cell.source_range[1]
                        {
                            return Err(Error::Invalid("nested row parent interval"));
                        }
                        cell_start = child.source_cp[1];
                        children.push(child);
                        continue;
                    }
                };
                let p = physical
                    .paragraphs
                    .get(index)
                    .ok_or(Error::Invalid("render cell paragraph"))?;
                if p.source_range[0] != cell_start
                    || p.source_range[1] > cell.source_range[1]
                    || !p.unresolved.is_empty()
                    || claimed[index]
                    || p.endpoint
                        .as_ref()
                        .is_none_or(|e| e.end_cp != p.source_range[1])
                {
                    return Err(Error::Unsupported("render cell physical ownership"));
                }
                let final_cell = p.source_range[1] == cell.source_range[1];
                let terminal = if final_cell && row.depth == 1 { 7 } else { 13 };
                if p.terminal_unit != Some(terminal)
                    || units[p.source_range[1] - 1] != terminal
                    || units[p.source_range[0]..p.source_range[1] - 1].contains(&7)
                {
                    return Err(Error::Unsupported("render cell terminal"));
                }
                let (style, direct, _) = properties_at_depth(
                    paragraphs,
                    p.source_range,
                    false,
                    row.depth,
                    final_cell,
                    &mut budget,
                )?;
                charge_style(styles, style, &mut style_budget)?;
                let resolved = resolve_cell_paragraph(
                    styles,
                    style,
                    &direct,
                    p.source_range,
                    pending.as_deref_mut(),
                )?;
                members.push(CellParagraph {
                    table_membership: nested::record_membership(
                        paragraphs,
                        p.source_range,
                        row.depth,
                    )?,
                    physical_index: index,
                    source_cp: p.source_range,
                    source_style: style,
                    properties: resolved,
                });
                claimed[index] = true;
                cell_start = p.source_range[1];
            }
            cells.push(SourceCell {
                source_cp: cell.source_range,
                paragraphs: members,
                nested_tables: group_rows(children)?,
            });
        }
        if cell_start != mark.source_range[0] || claimed[row.row_mark_paragraph] {
            return Err(Error::Invalid("render row source partition"));
        }
        let (style, direct, table) = properties_at_depth(
            paragraphs,
            mark.source_range,
            true,
            row.depth,
            true,
            &mut budget,
        )?;
        charge_style(styles, style, &mut style_budget)?;
        let row_mark_properties = paragraphs::resolve(styles, style, &direct)?;
        for index in std::iter::once(11usize).chain(
            table
                .iter()
                .filter(|p| p.sprm == 0x563a)
                .map(|p| crate::office::doc::u16_at(&p.operand, 0).map(usize::from))
                .collect::<Result<Vec<_>>>()?,
        ) {
            if table.iter().any(|p| p.sprm == 0x7621) {
                charge_style(styles, index, &mut style_budget)?;
            }
        }
        let layout = super::decode_row_with_styles(source.binary_version, &table, styles)?;
        if layout.cells.len() != cells.len() || row.definition_cell_count != Some(cells.len()) {
            return Err(Error::Invalid("render source cell count"));
        }
        claimed[row.row_mark_paragraph] = true;
        previous_end[row.depth] = row.source_range[1];
        rows.push(Some(SourceRow {
            table_membership: nested::record_membership(paragraphs, mark.source_range, row.depth)?,
            depth: row.depth,
            source_cp: row.source_range,
            row_mark_paragraph: row.row_mark_paragraph,
            row_mark_style: style,
            row_mark_properties,
            cells,
            layout,
        }));
    }
    // Every unclaimed paragraph must be an independently classified outside paragraph.
    for &index in &owned.outside_paragraphs {
        let slot = claimed
            .get_mut(index)
            .ok_or(Error::Invalid("render outside paragraph index"))?;
        if *slot {
            return Err(Error::Invalid("render duplicate paragraph ownership"));
        }
        *slot = true;
    }
    if claimed.contains(&false) {
        return Err(Error::Invalid("render unowned paragraph"));
    }
    rows.into_iter()
        .flatten()
        .map(|row| {
            if row.depth != 1 {
                return Err(Error::Invalid("nested row has no parent cell"));
            }
            Ok(row)
        })
        .collect()
}

// Charge every inherited style visit, including empty groups, before the resolver
// decodes it. Reused long ancestry chains therefore share one document work limit.
fn charge_style(styles: &Stylesheet, mut index: usize, budget: &mut usize) -> Result<()> {
    for _ in 0..4094 {
        let style = styles
            .styles
            .get(index)
            .and_then(Option::as_ref)
            .ok_or(Error::Invalid("render cell style"))?;
        *budget = budget
            .checked_sub(1)
            .ok_or(Error::Limit("render table style work"))?;
        for group in &style.properties {
            *budget = budget
                .checked_sub(group.bytes.len() + 1)
                .ok_or(Error::Limit("render table style work"))?;
        }
        match style.base_style {
            Some(base) => index = base,
            None => return Ok(()),
        }
    }
    Err(Error::Limit("render table style ancestry"))
}

#[cfg(test)]
fn properties(
    source: &ParagraphInspection,
    range: [usize; 2],
    row: bool,
    budget: &mut usize,
) -> Result<(usize, Vec<PropertyModifier>, Vec<PropertyModifier>)> {
    properties_at_depth(source, range, row, 1, false, budget)
}
fn properties_at_depth(
    source: &ParagraphInspection,
    range: [usize; 2],
    row: bool,
    depth: usize,
    final_cell: bool,
    budget: &mut usize,
) -> Result<(usize, Vec<PropertyModifier>, Vec<PropertyModifier>)> {
    let index = source.spans.partition_point(|p| p.end_cp <= range[0]);
    let span = source
        .spans
        .get(index)
        .ok_or(Error::Invalid("render row PAPX"))?;
    if [span.start_cp, span.end_cp] != range {
        return Err(Error::Invalid("render row PAPX endpoint"));
    }
    *budget = budget
        .checked_sub(span.raw_modifiers.len() + 1)
        .ok_or(Error::Limit("render row property bytes"))?;
    let mut ordinary = Vec::new();
    let mut table = Vec::new();
    let mut in_table = false;
    let mut row_mark = false;
    let mut inner_row = false;
    let mut inner_cell = false;
    let mut recorded_depth = 1usize;
    for p in decode(&span.raw_modifiers)? {
        match p.sprm {
            0x2416 => in_table = super::bool8(&p.operand)?,
            0x2417 => row_mark = super::bool8(&p.operand)?,
            0x6649 => {
                recorded_depth = usize::try_from(crate::office::doc::u32_at(&p.operand, 0)?)
                    .map_err(|_| Error::Invalid("table depth"))?;
                if p.operand.len() != 4 || recorded_depth != depth {
                    return Err(Error::Invalid("table depth source mismatch"));
                }
            }
            0x244b if depth > 1 => inner_cell = super::bool8(&p.operand)?,
            0x244c if depth > 1 => inner_row = super::bool8(&p.operand)?,
            op if (op >> 10) & 7 == 5 && row => table.push(p),
            op if (op >> 10) & 7 == 1 => ordinary.push(p),
            _ => return Err(Error::Unsupported("render cell property scope")),
        }
    }
    if !in_table
        || (depth == 1 && row_mark != row)
        || (depth > 1
            && (recorded_depth != depth
                || row_mark
                || inner_row != row
                || inner_cell != final_cell))
    {
        return Err(Error::Invalid("render cell membership properties"));
    }
    Ok((span.style_index.unwrap_or(0) as usize, ordinary, table))
}

#[cfg(test)]
mod tests;
