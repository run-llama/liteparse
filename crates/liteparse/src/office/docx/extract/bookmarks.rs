use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use crate::office::pptx::scene::{Item, SourceBookmark};
use serde_json::{Value, json};

use crate::office::docx::paginate::OutPage;

/// Built once from accepted pages; source names and canonical positions use indexed lookups.
pub(super) struct Index {
    definitions: HashMap<String, Vec<Arc<SourceBookmark>>>,
    canonical: HashMap<String, Value>,
    occurrences: Vec<Vec<Value>>,
}

impl Index {
    pub fn new(definitions: &[Arc<SourceBookmark>], pages: &[OutPage]) -> Self {
        if definitions.is_empty() {
            return Self {
                definitions: HashMap::new(),
                canonical: HashMap::new(),
                occurrences: vec![Vec::new(); pages.len()],
            };
        }
        let mut names: HashMap<String, Vec<Arc<SourceBookmark>>> = HashMap::new();
        for marker in definitions {
            names
                .entry(marker.name.clone())
                .or_default()
                .push(marker.clone());
        }
        let mut canonical = HashMap::new();
        let mut occurrences = Vec::with_capacity(pages.len());
        let mut ordinal = 0;
        let mut referenced_names = HashSet::new();
        for (page_index, page) in pages.iter().enumerate() {
            let mut items: Vec<_> = page.items.iter().collect();
            items.sort_by_key(|item| match item {
                Item::Text(run) => run.paragraph,
                Item::Bookmark(marker) => marker.reading_key,
                _ => 0,
            });
            let mut page_markers = Vec::new();
            let mut add = |marker: &SourceBookmark, point: tiny_skia::Point, standalone: bool| {
                if !point.x.is_finite()
                    || !point.y.is_finite()
                    || point.x < 0.0
                    || point.y < 0.0
                    || point.x as f64 > page.info.w
                    || point.y as f64 > page.info.h
                {
                    return;
                }
                let mut value = crate::office::pptx::output::bookmark_json(marker, point);
                value["pageIndex"] = json!(page_index);
                value["occurrenceIndex"] = json!(ordinal);
                ordinal += 1;
                value["canonical"] = json!(!canonical.contains_key(&marker.id));
                canonical
                    .entry(marker.id.clone())
                    .or_insert_with(|| value.clone());
                value["standalone"] = json!(standalone);
                page_markers.push(value);
            };
            for item in items {
                match item {
                    Item::Bookmark(marker) => add(&marker.source, marker.point, true),
                    Item::Text(run) => {
                        if run.text.is_empty()
                            || (run.text.trim().is_empty() && !run.preserve_whitespace)
                        {
                            continue;
                        }
                        let Some(bounds) = crate::office::pptx::textlayout::transformed_bbox(
                            &run.transform,
                            0.0,
                            -run.ascent,
                            run.width,
                            run.ascent + run.descent,
                        ) else {
                            continue;
                        };
                        if bounds.x >= page.info.w
                            || bounds.y >= page.info.h
                            || bounds.x + bounds.w <= 0.0
                            || bounds.y + bounds.h <= 0.0
                        {
                            continue;
                        }
                        let mut reference = |source: Option<
                            &crate::office::pptx::scene::TextSource,
                        >| {
                            if let Some(crate::office::pptx::scene::SourceLink {
                                target:
                                    crate::office::pptx::scene::SourceLinkTarget::Bookmark {
                                        name, ..
                                    },
                                ..
                            }) = source.and_then(|source| source.link.as_ref())
                            {
                                referenced_names.insert(name.clone());
                            }
                        };
                        let spans = run
                            .bookmarks
                            .as_ref()
                            .map(|metadata| metadata.source_spans.as_slice())
                            .unwrap_or(&[]);
                        if !spans.is_empty() {
                            for span in spans.iter() {
                                if let Some(bounds) =
                                    crate::office::pptx::textlayout::transformed_bbox(
                                        &run.transform,
                                        span.x,
                                        -run.ascent,
                                        span.width,
                                        run.ascent + run.descent,
                                    )
                                    && bounds.x < page.info.w
                                    && bounds.y < page.info.h
                                    && bounds.x + bounds.w > 0.0
                                    && bounds.y + bounds.h > 0.0
                                {
                                    reference(span.source.as_deref());
                                }
                            }
                        } else {
                            reference(run.source.as_deref());
                        }
                        for marker in run
                            .bookmarks
                            .iter()
                            .flat_map(|bookmarks| &bookmarks.boundaries)
                        {
                            let mut point = marker.point;
                            run.transform.map_point(&mut point);
                            add(&marker.source, point, false);
                        }
                    }
                    _ => {}
                }
            }
            occurrences.push(page_markers);
        }
        let referenced_ids: HashSet<_> = referenced_names
            .iter()
            .filter_map(|name| {
                let definitions = names.get(name)?;
                (definitions.len() == 1 && canonical.contains_key(&definitions[0].id))
                    .then(|| definitions[0].id.clone())
            })
            .collect();
        for occurrence in occurrences.iter_mut().flatten() {
            occurrence["referenced"] = json!(
                occurrence["id"]
                    .as_str()
                    .is_some_and(|id| referenced_ids.contains(id))
            );
        }
        for (id, occurrence) in &mut canonical {
            occurrence["referenced"] = json!(referenced_ids.contains(id));
        }
        Self {
            definitions: names,
            canonical,
            occurrences,
        }
    }

    fn resolve(&self, destination: &mut Value) {
        let name = destination["bookmarkName"].as_str().unwrap_or("");
        let Some(definitions) = self.definitions.get(name) else {
            destination["status"] = json!("missing");
            return;
        };
        if definitions.len() != 1 {
            destination["status"] = json!("ambiguous");
            return;
        }
        let marker = &definitions[0];
        destination["id"] = json!(marker.id);
        destination["targetPart"] = json!(marker.source.part);
        if let Some(placed) = self.canonical.get(&marker.id) {
            destination["status"] = json!("resolved");
            for key in ["pageIndex", "point", "occurrenceIndex"] {
                destination[key] = placed[key].clone();
            }
        } else {
            destination["status"] = json!("unplaced");
        }
    }

    pub fn resolve_text(&self, text: &mut Value) {
        for (mcid, item) in text["items"]
            .as_array_mut()
            .into_iter()
            .flatten()
            .enumerate()
        {
            if !self.definitions.is_empty() {
                item["mcid"] = json!(mcid);
            }
            if let Some(destination) = item.pointer_mut("/nativeSource/link/destination") {
                self.resolve(destination);
            }
        }
    }

    pub fn decorate(&self, tree: &mut Value, page: u32) {
        let occurrences = &self.occurrences[page as usize];
        if occurrences.is_empty() {
            if let Some(object) = tree.as_object_mut() {
                object.remove("destinations");
            }
            return;
        }
        let key = |value: &Value| {
            (
                value["id"].as_str().unwrap_or("").to_owned(),
                value["point"].to_string(),
            )
        };
        let mut by_boundary: HashMap<_, VecDeque<&Value>> = HashMap::new();
        for occurrence in occurrences {
            by_boundary
                .entry(key(occurrence))
                .or_default()
                .push_back(occurrence);
        }
        let mut result = Vec::new();
        for marker in tree["destinations"].as_array().into_iter().flatten() {
            if let Some(candidate) = by_boundary
                .get_mut(&key(marker))
                .and_then(VecDeque::pop_front)
            {
                let mut value = candidate.clone();
                for field in ["mcid", "utf16Offset", "textItemBoundary"] {
                    if let Some(boundary) = marker.get(field) {
                        value[field] = boundary.clone();
                    }
                }
                result.push(value);
            }
        }
        result.sort_by_key(|marker| marker["occurrenceIndex"].as_u64().unwrap_or(0));
        for marker in &mut result {
            marker.as_object_mut().unwrap().remove("standalone");
        }
        if result.is_empty() {
            tree.as_object_mut().unwrap().remove("destinations");
        } else {
            tree["destinations"] = json!(result);
        }
    }
}

#[cfg(test)]
mod tests;
