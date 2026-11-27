use super::*;

#[test]
fn occupied_merge_preview_preserves_extent_and_discharges_consumed_debt() {
    let slices = [(0.0, 80.0), (12.0, 12.0), (90.0, 100.0), (102.0, 12.0)]
        .into_iter()
        .map(|(y, h)| Slice {
            y,
            h,
            items: Vec::new(),
            anchors: Vec::new(),
            paragraph_offset: 0.0,
            widow: None,
            notes: Vec::new(),
            nested_row: None,
            nested_table: None,
            paragraph_source: None,
        })
        .collect();
    let mut batch = Batch {
        valid: true,
        cursors: vec![Cursor {
            row_sources: Default::default(),
            source_width: 100.0,
            source_offset: 0,
            reading_offset: 0,
            cell: 0,
            source_cell: 0,
            x: 0.0,
            margins: [0.0; 4],
            source: Rc::new((slices, 190.0)),
            next: 0,
            start: 0.0,
            part: Some(Part {
                source_row: 0,
                cell_geometry: None,
                page: 0,
                x: 0.0,
                top: 0.0,
                bottom: 0.0,
                first: 0,
                last: 0,
                fresh: true,
            }),
            nested: true,
            restart: (0, 0),
            fresh_capacity: 100.0,
        }],
        pending: Vec::new(),
    };
    assert_eq!(batch.required(0.0), 190.0);
    let preview = batch.preview(0.0, 85.0);
    assert_eq!(preview.top, 80.0);
    assert_eq!(preview.rest, 100.0);
    batch.commit(0.0, 85.0, 0);
    assert_eq!(batch.cursors[0].next, 2);
    batch.prepare_note(1, 0.0, 0.0, 1, true);
    assert_eq!(batch.required(0.0), 100.0);
    let preview = batch.preview(0.0, 100.0);
    assert_eq!(preview.top, 100.0);
    assert_eq!(preview.rest, 0.0);
    batch.commit(0.0, 100.0, 1);
    assert!(batch.complete());
    assert_eq!(batch.required(0.0), 0.0);
}
