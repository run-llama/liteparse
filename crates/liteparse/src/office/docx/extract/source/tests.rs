use super::*;

#[test]
fn merged_owner_uses_accepted_row_without_a_phantom_source_row() {
    let merge = json!({"cell":0,"gridStart":0,"span":1,"startRow":0,"endRow":1});
    let row = |index, cell| json!({"table":0,"row":index,"cell":cell,"span":1,"cellCount":2,"cellSpans":[1,1],"textOnly":true,"verticalMerges":[merge.clone()]});
    let mut owner = row(0, 0);
    owner["fragment"] = row(1, 0);
    let mut text = json!({"items":[
        {"unicode":"TAIL","nativeSource":{"part":"word/document.xml","paragraph":1,"cells":[owner]}},
        {"unicode":"RIGHT","nativeSource":{"part":"word/document.xml","paragraph":2,"cells":[row(1,1)]}}
    ]});
    let tree = structure(&mut text);
    let rows = tree["children"][0]["children"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["sourceRowIndex"], 1);
    assert_eq!(rows[0]["children"][0]["sourceOwnerRowIndex"], 0);
    assert_eq!(rows[0]["children"][0]["markedContentIds"], json!([0]));
    assert_eq!(rows[0]["children"][1]["markedContentIds"], json!([1]));
    assert_eq!(rows[0]["sourceVerticalMerges"], json!([merge]));
}

#[test]
fn accepted_fragment_grid_offsets_override_source_owner_row_offsets() {
    let owner = json!({"table":0,"row":0,"cell":0,"span":1,"cellCount":1,"cellSpans":[1],"gridBefore":2,"textOnly":true,
        "fragment":{"table":0,"row":1,"cell":0,"span":1,"cellCount":1,"cellSpans":[1],"gridBefore":1,"gridAfter":3,"textOnly":true}});
    let mut text = json!({"items":[{"unicode":"TAIL","nativeSource":{"part":"word/document.xml","paragraph":1,"cells":[owner]}}]});
    let tree = structure(&mut text);
    let row = &tree["children"][0]["children"][0];
    assert_eq!(row["sourceGridBefore"], 1);
    assert_eq!(row["sourceGridAfter"], 3);
    assert_eq!(row["children"][0]["sourceOwnerRowIndex"], 0);
}
