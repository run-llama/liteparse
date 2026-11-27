//! Checked logical table columns shared by layout and source ownership.

use super::*;
use crate::office::pptx::error::{DocError, DocResult};

pub(super) struct RowGrid {
    pub before: usize,
    pub after: usize,
    pub spans: Vec<usize>,
}

pub(super) struct Budget {
    max_columns: usize,
    remaining_work: usize,
    failure: Option<String>,
}

impl Budget {
    pub fn document() -> Self {
        // One dense f64 grid is at most 512 KiB; work covers all parsed stories.
        Self::new(65_536, 16_777_216)
    }

    pub fn new(max_columns: usize, work: usize) -> Self {
        Self {
            max_columns,
            remaining_work: work,
            failure: None,
        }
    }

    pub fn check(&self) -> DocResult<()> {
        match &self.failure {
            Some(detail) => Err(DocError::LimitExceeded(detail.clone())),
            None => Ok(()),
        }
    }

    pub fn table(&mut self, table: &XNode, rows: &[&Rc<XNode>]) -> Option<Vec<RowGrid>> {
        if self.failure.is_some() {
            return None;
        }
        match self.validate(table, rows) {
            Ok(rows) => Some(rows),
            Err(()) => {
                self.failure =
                    Some("table logical grid exceeds the column or cumulative work budget".into());
                None
            }
        }
    }

    fn validate(&mut self, table: &XNode, rows: &[&Rc<XNode>]) -> Result<Vec<RowGrid>, ()> {
        let mut columns = table
            .child("w:tblGrid")
            .map_or(0, |grid| grid.children_named("w:gridCol").count());
        if columns > self.max_columns {
            return Err(());
        }
        let mut result = Vec::with_capacity(rows.len());
        let mut cells = 0usize;
        let mut span_work = 0usize;
        for row in rows {
            let pr = row.child("w:trPr");
            let before = count(pr.and_then(|p| p.child("w:gridBefore")), 0)?;
            let after = count(pr.and_then(|p| p.child("w:gridAfter")), 0)?;
            let mut nodes = Vec::new();
            collect_named(&row.children, "w:tc", &mut nodes);
            cells = cells.checked_add(nodes.len()).ok_or(())?;
            let mut spans = Vec::with_capacity(nodes.len());
            let mut endpoint = before;
            for cell in nodes {
                let span = count(cell.child("w:tcPr").and_then(|p| p.child("w:gridSpan")), 1)?;
                endpoint = endpoint.checked_add(span).ok_or(())?;
                span_work = span_work.checked_add(span).ok_or(())?;
                if endpoint > self.max_columns {
                    return Err(());
                }
                spans.push(span);
            }
            endpoint = endpoint.checked_add(after).ok_or(())?;
            columns = columns.max(endpoint);
            if columns > self.max_columns {
                return Err(());
            }
            result.push(RowGrid {
                before,
                after,
                spans,
            });
        }
        // Account for dense row/grid storage and column-by-cell constraint scans.
        // The projection's iteration limit and repeated layout remain separate costs.
        let work = rows
            .len()
            .checked_add(cells)
            .and_then(|v| v.checked_add(1))
            .and_then(|v| columns.checked_mul(v))
            .and_then(|v| v.checked_add(span_work))
            .ok_or(())?;
        self.remaining_work = self.remaining_work.checked_sub(work).ok_or(())?;
        Ok(result)
    }
}

/// Preserve existing malformed/negative normalization while rejecting positive overflow.
fn count(node: Option<&Rc<XNode>>, minimum: usize) -> Result<usize, ()> {
    let Some(raw) = node.and_then(|n| n.attr("val")) else {
        return Ok(minimum);
    };
    let raw = raw.trim();
    let positive = raw.strip_prefix('+').unwrap_or(raw);
    if !positive.is_empty() && positive.bytes().all(|b| b.is_ascii_digit()) {
        return positive
            .parse::<usize>()
            .map(|n| n.max(minimum))
            .map_err(|_| ());
    }
    Ok(minimum)
}

#[cfg(test)]
mod tests;
