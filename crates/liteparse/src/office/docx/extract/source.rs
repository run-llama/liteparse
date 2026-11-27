use serde_json::{Value, json};

fn child<'a>(parent: &'a mut Value, id: &str, node: Value) -> &'a mut Value {
    let children = parent["children"]
        .as_array_mut()
        .expect("structure children");
    let index = children
        .iter()
        .position(|child| child["id"] == id)
        .unwrap_or_else(|| {
            children.push(node);
            children.len() - 1
        });
    &mut children[index]
}

/// Native content IDs identify final exported items after clipping and duplicate removal.
pub(super) fn structure(text: &mut Value) -> Value {
    let mut root = json!({"type": "Document", "nativeSource": true, "children": []});
    if let Some(markers) = text
        .as_object_mut()
        .and_then(|text| text.remove("nativeStandaloneBookmarks"))
    {
        root["destinations"] = markers;
    }
    let Some(items) = text["items"].as_array_mut() else {
        return root;
    };
    for (mcid, item) in items.iter_mut().enumerate() {
        if let Some(markers) = item
            .as_object_mut()
            .and_then(|item| item.remove("nativeBookmarks"))
        {
            item["mcid"] = json!(mcid);
            for mut marker in markers.as_array().into_iter().flatten().cloned() {
                marker["mcid"] = json!(mcid);
                if root.get("destinations").is_none() {
                    root["destinations"] = json!([]);
                }
                root["destinations"].as_array_mut().unwrap().push(marker);
            }
        }
        let Some(source) = item.get("nativeSource").cloned() else {
            continue;
        };
        item["mcid"] = json!(mcid);
        let part = source["part"].as_str().unwrap_or("");
        let mut parent = &mut root;
        if let Some(cell) = source["cells"].as_array().and_then(|cells| cells.first()) {
            let owner = cell;
            let cell = cell.get("fragment").unwrap_or(cell);
            let table = cell["table"].as_u64().unwrap_or(0);
            let row = cell["row"].as_u64().unwrap_or(0);
            let column = cell["cell"].as_u64().unwrap_or(0);
            let id = format!("{part}:table-{table}");
            parent = child(
                parent,
                &id,
                json!({"type":"Table", "id":id, "sourcePart":part, "sourceTableId":table, "sourceTextOnly":cell["textOnly"], "children":[]}),
            );
            let id = format!("{id}:row-{row}");
            parent = child(
                parent,
                &id,
                json!({"type":"TR", "id":id, "sourceRowIndex":row, "sourceCellSpans":cell["cellSpans"], "children":[]}),
            );
            for (source, target) in [
                ("gridBefore", "sourceGridBefore"),
                ("gridAfter", "sourceGridAfter"),
            ] {
                if let Some(offset) = cell.get(source) {
                    parent[target] = offset.clone();
                }
            }
            if let Some(merges) = cell.get("verticalMerges") {
                parent["sourceVerticalMerges"] = merges.clone();
            }
            let id = format!("{id}:cell-{column}");
            parent = child(
                parent,
                &id,
                json!({"type":"TD", "id":id, "sourceCellIndex":column, "sourceColSpan":cell["span"], "sourceCellCount":cell["cellCount"], "markedContentIds":[], "children":[]}),
            );
            if owner.get("fragment").is_some() {
                parent["sourceOwnerRowIndex"] = owner["row"].clone();
            }
            parent["markedContentIds"]
                .as_array_mut()
                .expect("cell content")
                .push(json!(mcid));
            let paragraph = source["paragraph"].as_u64().unwrap_or(0);
            let id = format!("{id}:paragraph-{paragraph}");
            parent = child(
                parent,
                &id,
                json!({"type":"P", "id":id, "sourceParagraphId":paragraph, "markedContentIds":[], "children":[]}),
            );
            parent["markedContentIds"]
                .as_array_mut()
                .expect("paragraph content")
                .push(json!(mcid));
        }
        if let Some(link) = source["link"].as_object() {
            let link_id = link.get("id").and_then(Value::as_str).unwrap_or("");
            let annotation = if let Some(uri) = link.get("uri") {
                json!({"subtype":"Link","uri":uri})
            } else {
                json!({"subtype":"Link","destination":link.get("destination")})
            };
            let id = format!("{part}:{link_id}");
            let link = child(
                parent,
                &id,
                json!({"type":"Link", "id":id, "sourcePart":part, "sourceLinkId":link_id, "annotations":[annotation], "markedContentIds":[], "children":[]}),
            );
            link["markedContentIds"]
                .as_array_mut()
                .expect("link content")
                .push(json!(mcid));
        }
    }
    root
}

#[cfg(test)]
mod tests;
