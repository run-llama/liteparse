//! Walk the PDF structure tree (tagged-PDF tree) for a page.
//!
//! Each struct element has:
//! - a role (string: "H1", "Figure", "Table", ...)
//! - zero or more marked content ids (mcids) that tie the element to
//!   page-content stream blocks
//! - zero or more child elements
//!
//! For LiteParse's purposes we want a flat list of nodes with:
//! - the role string
//! - viewport-space bbox derived from joining page objects whose
//!   `FPDFPageObj_GetMarkedContentID` matches one of this node's mcids
//! - the mcid list itself (so the layout pass can map text runs to nodes)

use crate::ffi;
use crate::page::{Page, ViewportTransform};
use crate::types::RectF;
use std::collections::BTreeMap;

/// A scalar value from a structure element's PDF `/A` attribute dictionary.
#[derive(Debug, Clone, PartialEq)]
pub enum StructureAttributeValue {
    Boolean(bool),
    Number(f32),
    String(String),
}

/// One element in the page-scoped tagged-PDF structure tree.
#[derive(Debug, Clone)]
pub struct StructureElement {
    pub element_type: String,
    pub id: Option<String>,
    pub actual_text: Option<String>,
    pub alt_text: Option<String>,
    pub title: Option<String>,
    pub attributes: BTreeMap<String, StructureAttributeValue>,
    pub marked_content_ids: Vec<i32>,
    pub children: Vec<StructureElement>,
    /// PDF object numbers for link annotations referenced by non-element children.
    pub annotation_object_numbers: Vec<i32>,
}

/// The page-scoped structure tree with the root count pdfium reported.
///
/// `root_count` can exceed `roots.len()`: pdfium returns a null element for a
/// root that does not belong to the page, and those are skipped. Callers that
/// shape output on "one root vs several" need the declared count, not the
/// number that resolved.
#[derive(Debug, Clone, Default)]
pub struct StructureTree {
    pub root_count: usize,
    pub roots: Vec<StructureElement>,
}

/// One node in the structure tree, flattened for downstream use.
#[derive(Debug, Clone)]
pub struct StructNode {
    /// Role string from `FPDF_StructElement_GetType` (e.g. "H1", "P", "Figure").
    pub role: String,
    /// Marked content ids attached to this element (and its non-struct child markers).
    pub mcids: Vec<i32>,
    /// Union bbox of page objects tagged with any of `mcids`, in viewport
    /// coordinates (top-left origin, 72 DPI). `None` when none of the mcids
    /// resolved to a bbox on the page.
    pub bbox: Option<RectF>,
    /// Optional alt text (set for Figure / Formula elements when present).
    pub alt_text: Option<String>,
    /// Nesting depth in the tree (page roots are 0). With pre-order this
    /// recovers each node's subtree: the following nodes deeper than it.
    pub depth: u16,
}

impl Page<'_, '_> {
    /// Extract the complete tagged-PDF structure tree for public output.
    /// Returns an empty vector for untagged pages and preserves multiple roots.
    pub fn structure_tree(&self) -> Vec<StructureElement> {
        self.structure_tree_with_root_count().roots
    }

    /// [`Page::structure_tree`] plus the number of roots pdfium declared for
    /// the page, including the ones it could not resolve.
    pub fn structure_tree_with_root_count(&self) -> StructureTree {
        let tree = unsafe { ffi!(FPDF_StructTree_GetForPage(self.handle)) };
        if tree.is_null() {
            return StructureTree::default();
        }
        let count = unsafe { ffi!(FPDF_StructTree_CountChildren(tree)) };
        let mut roots = Vec::with_capacity(count.max(0) as usize);
        for index in 0..count {
            let element = unsafe { ffi!(FPDF_StructTree_GetChildAtIndex(tree, index)) };
            if !element.is_null() {
                roots.push(read_structure_element(element));
            }
        }
        unsafe { ffi!(FPDF_StructTree_Close(tree)) };
        StructureTree {
            root_count: count.max(0) as usize,
            roots,
        }
    }

    /// Walk this page's structure tree (tagged-PDF tree). Returns an empty
    /// vec when the page is untagged or the document has no struct tree.
    /// Nodes are returned in pre-order (parent before children).
    pub fn struct_tree(&self, view_box: &RectF) -> Vec<StructNode> {
        let tree = unsafe { ffi!(FPDF_StructTree_GetForPage(self.handle)) };
        if tree.is_null() {
            return Vec::new();
        }

        let mcid_bboxes = collect_mcid_bboxes(self, view_box);
        let mut out = Vec::new();

        let count = unsafe { ffi!(FPDF_StructTree_CountChildren(tree)) };
        for i in 0..count {
            let elem = unsafe { ffi!(FPDF_StructTree_GetChildAtIndex(tree, i)) };
            if !elem.is_null() {
                walk_element(elem, &mcid_bboxes, 0, &mut out);
            }
        }

        unsafe { ffi!(FPDF_StructTree_Close(tree)) };

        out
    }
}

fn read_structure_element(elem: pdfium_sys::FPDF_STRUCTELEMENT) -> StructureElement {
    let element_type = read_element_type(elem);
    let id = read_optional_widestring(|buf, len| unsafe {
        ffi!(FPDF_StructElement_GetID(elem, buf, len))
    });
    let actual_text = read_optional_widestring(|buf, len| unsafe {
        ffi!(FPDF_StructElement_GetActualText(elem, buf, len))
    });
    let alt_text = read_alt_text(elem);
    let title = read_optional_widestring(|buf, len| unsafe {
        ffi!(FPDF_StructElement_GetTitle(elem, buf, len))
    });
    let attributes = read_attributes(elem);
    let marked_content_ids = read_marked_content_ids(elem);
    let count = unsafe { ffi!(FPDF_StructElement_CountChildren(elem)) };
    let mut children = Vec::new();
    let mut annotation_object_numbers = Vec::new();
    for index in 0..count {
        let child = unsafe { ffi!(FPDF_StructElement_GetChildAtIndex(elem, index)) };
        if child.is_null() {
            let object_number = unsafe { ffi!(FPDF_StructElement_GetChildObjNum(elem, index)) };
            if object_number >= 0 {
                annotation_object_numbers.push(object_number);
            }
        } else {
            children.push(read_structure_element(child));
        }
    }
    StructureElement {
        element_type,
        id,
        actual_text,
        alt_text,
        title,
        attributes,
        marked_content_ids,
        children,
        annotation_object_numbers,
    }
}

fn read_marked_content_ids(elem: pdfium_sys::FPDF_STRUCTELEMENT) -> Vec<i32> {
    let count = unsafe { ffi!(FPDF_StructElement_GetMarkedContentIdCount(elem)) };
    let mut ids = Vec::with_capacity(count.max(0) as usize);
    for index in 0..count {
        let id = unsafe { ffi!(FPDF_StructElement_GetMarkedContentIdAtIndex(elem, index)) };
        if id >= 0 {
            ids.push(id);
        }
    }
    ids
}

fn read_attributes(
    elem: pdfium_sys::FPDF_STRUCTELEMENT,
) -> BTreeMap<String, StructureAttributeValue> {
    let mut out = BTreeMap::new();
    let attr_count = unsafe { ffi!(FPDF_StructElement_GetAttributeCount(elem)) };
    for attr_index in 0..attr_count {
        let attr = unsafe { ffi!(FPDF_StructElement_GetAttributeAtIndex(elem, attr_index)) };
        if attr.is_null() {
            continue;
        }
        let value_count = unsafe { ffi!(FPDF_StructElement_Attr_GetCount(attr)) };
        for value_index in 0..value_count {
            let Some(name) = read_attribute_name(attr, value_index) else {
                continue;
            };
            let mut name_bytes = name.as_bytes().to_vec();
            name_bytes.push(0);
            let value = unsafe {
                ffi!(FPDF_StructElement_Attr_GetValue(
                    attr,
                    name_bytes.as_ptr().cast()
                ))
            };
            if value.is_null() {
                continue;
            }
            let value_type = unsafe { ffi!(FPDF_StructElement_Attr_GetType(value)) };
            let parsed = if value_type == pdfium_sys::FPDF_OBJECT_BOOLEAN as i32 {
                let mut boolean = 0;
                (unsafe { ffi!(FPDF_StructElement_Attr_GetBooleanValue(value, &mut boolean)) } != 0)
                    .then_some(StructureAttributeValue::Boolean(boolean != 0))
            } else if value_type == pdfium_sys::FPDF_OBJECT_NUMBER as i32 {
                let mut number = 0.0;
                (unsafe { ffi!(FPDF_StructElement_Attr_GetNumberValue(value, &mut number)) } != 0)
                    .then_some(StructureAttributeValue::Number(number))
            } else if value_type == pdfium_sys::FPDF_OBJECT_STRING as i32
                || value_type == pdfium_sys::FPDF_OBJECT_NAME as i32
            {
                read_attribute_string(value).map(StructureAttributeValue::String)
            } else {
                None
            };
            if let Some(parsed) = parsed {
                out.insert(name, parsed);
            }
        }
    }
    out
}

fn read_attribute_name(attr: pdfium_sys::FPDF_STRUCTELEMENT_ATTR, index: i32) -> Option<String> {
    let mut needed = 0;
    unsafe {
        ffi!(FPDF_StructElement_Attr_GetName(
            attr,
            index,
            std::ptr::null_mut(),
            0,
            &mut needed
        ));
    }
    if needed == 0 || needed > usize::MAX as std::os::raw::c_ulong {
        return None;
    }
    let mut bytes = vec![0u8; needed as usize];
    let ok = unsafe {
        ffi!(FPDF_StructElement_Attr_GetName(
            attr,
            index,
            bytes.as_mut_ptr().cast(),
            needed,
            &mut needed
        ))
    };
    if ok == 0 {
        return None;
    }
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    String::from_utf8(bytes[..end].to_vec()).ok()
}

fn read_attribute_string(value: pdfium_sys::FPDF_STRUCTELEMENT_ATTR_VALUE) -> Option<String> {
    let mut needed = 0;
    unsafe {
        ffi!(FPDF_StructElement_Attr_GetStringValue(
            value,
            std::ptr::null_mut(),
            0,
            &mut needed
        ));
    }
    if needed < 2 || needed > usize::MAX as std::os::raw::c_ulong {
        return None;
    }
    let mut buffer = vec![0u16; needed as usize / 2];
    let ok = unsafe {
        ffi!(FPDF_StructElement_Attr_GetStringValue(
            value,
            buffer.as_mut_ptr().cast(),
            needed,
            &mut needed
        ))
    };
    if ok == 0 {
        return None;
    }
    let chars = needed as usize / 2;
    let mut end = chars.min(buffer.len());
    while end > 0 && buffer[end - 1] == 0 {
        end -= 1;
    }
    Some(String::from_utf16_lossy(&buffer[..end]))
}

fn read_optional_widestring<F>(getter: F) -> Option<String>
where
    F: Fn(*mut std::os::raw::c_void, std::os::raw::c_ulong) -> std::os::raw::c_ulong,
{
    let value = read_widestring(getter);
    (!value.is_empty()).then_some(value)
}

/// Pre-scan all page objects on the page, building `mcid → union(bbox)` in
/// viewport space. Each struct node then unions the bboxes for its own mcids.
fn collect_mcid_bboxes(
    page: &Page<'_, '_>,
    view_box: &RectF,
) -> std::collections::HashMap<i32, RectF> {
    let vp = page.viewport_transform(view_box);
    let obj_count = unsafe { ffi!(FPDFPage_CountObjects(page.handle)) };
    let mut map: std::collections::HashMap<i32, RectF> = std::collections::HashMap::new();

    for i in 0..obj_count {
        let obj = unsafe { ffi!(FPDFPage_GetObject(page.handle, i)) };
        if obj.is_null() {
            continue;
        }
        let mcid = unsafe { ffi!(FPDFPageObj_GetMarkedContentID(obj)) };
        if mcid < 0 {
            continue;
        }
        let bbox = page_object_bbox(obj, &vp);
        if let Some(b) = bbox {
            map.entry(mcid)
                .and_modify(|cur| *cur = union_rect(cur, &b))
                .or_insert(b);
        }
    }

    map
}

fn page_object_bbox(obj: pdfium_sys::FPDF_PAGEOBJECT, vp: &ViewportTransform) -> Option<RectF> {
    let mut left = 0.0f32;
    let mut bottom = 0.0f32;
    let mut right = 0.0f32;
    let mut top = 0.0f32;
    let ok = unsafe {
        ffi!(FPDFPageObj_GetBounds(
            obj,
            &mut left,
            &mut bottom,
            &mut right,
            &mut top
        ))
    };
    if ok == 0 {
        return None;
    }
    Some(vp.transform_bounds(&RectF {
        left,
        top,
        right,
        bottom,
    }))
}

fn union_rect(a: &RectF, b: &RectF) -> RectF {
    RectF {
        left: a.left.min(b.left),
        top: a.top.min(b.top),
        right: a.right.max(b.right),
        bottom: a.bottom.max(b.bottom),
    }
}

fn walk_element(
    elem: pdfium_sys::FPDF_STRUCTELEMENT,
    mcid_bboxes: &std::collections::HashMap<i32, RectF>,
    depth: u16,
    out: &mut Vec<StructNode>,
) {
    let role = read_element_type(elem);
    let alt_text = read_alt_text(elem);

    // Collect mcids: the multi-mcid getters + per-child marked-content-only children.
    let mut mcids: Vec<i32> = Vec::new();
    let n_mcids = unsafe { ffi!(FPDF_StructElement_GetMarkedContentIdCount(elem)) };
    for i in 0..n_mcids {
        let m = unsafe { ffi!(FPDF_StructElement_GetMarkedContentIdAtIndex(elem, i)) };
        if m >= 0 {
            mcids.push(m);
        }
    }
    // Also legacy: single direct mcid getter (older tag-trees expose it here).
    let single = unsafe { ffi!(FPDF_StructElement_GetMarkedContentID(elem)) };
    if single >= 0 && !mcids.contains(&single) {
        mcids.push(single);
    }

    let n_children = unsafe { ffi!(FPDF_StructElement_CountChildren(elem)) };
    for i in 0..n_children {
        // Non-struct children expose their mcid via GetChildMarkedContentID
        // (returns -1 when the child is itself a struct element).
        let child_mcid = unsafe { ffi!(FPDF_StructElement_GetChildMarkedContentID(elem, i)) };
        if child_mcid >= 0 && !mcids.contains(&child_mcid) {
            mcids.push(child_mcid);
        }
    }

    let bbox = union_mcid_bboxes(&mcids, mcid_bboxes);
    out.push(StructNode {
        role,
        mcids,
        bbox,
        alt_text,
        depth,
    });

    for i in 0..n_children {
        let child_elem = unsafe { ffi!(FPDF_StructElement_GetChildAtIndex(elem, i)) };
        if !child_elem.is_null() {
            walk_element(child_elem, mcid_bboxes, depth.saturating_add(1), out);
        }
    }
}

fn union_mcid_bboxes(
    mcids: &[i32],
    mcid_bboxes: &std::collections::HashMap<i32, RectF>,
) -> Option<RectF> {
    let mut acc: Option<RectF> = None;
    for m in mcids {
        if let Some(b) = mcid_bboxes.get(m) {
            acc = Some(match acc {
                Some(a) => union_rect(&a, b),
                None => *b,
            });
        }
    }
    acc
}

fn read_element_type(elem: pdfium_sys::FPDF_STRUCTELEMENT) -> String {
    read_widestring(|buf, len| unsafe { ffi!(FPDF_StructElement_GetType(elem, buf, len)) })
}

fn read_alt_text(elem: pdfium_sys::FPDF_STRUCTELEMENT) -> Option<String> {
    let s =
        read_widestring(|buf, len| unsafe { ffi!(FPDF_StructElement_GetAltText(elem, buf, len)) });
    if s.is_empty() { None } else { Some(s) }
}

/// Read a PDFium UTF-16LE widestring out-param via the "call once for size,
/// allocate, call again" pattern. `getter` is `(buf, buflen) -> bytes_written`.
fn read_widestring<F>(getter: F) -> String
where
    F: Fn(*mut std::os::raw::c_void, std::os::raw::c_ulong) -> std::os::raw::c_ulong,
{
    let needed = getter(std::ptr::null_mut(), 0) as usize;
    if needed < 2 {
        return String::new();
    }
    let mut buf: Vec<u16> = vec![0; needed / 2];
    let written = getter(
        buf.as_mut_ptr() as *mut std::os::raw::c_void,
        needed as std::os::raw::c_ulong,
    ) as usize;
    if written < 2 {
        return String::new();
    }
    let chars = written / 2;
    let mut end = chars.min(buf.len());
    while end > 0 && buf[end - 1] == 0 {
        end -= 1;
    }
    String::from_utf16_lossy(&buf[..end])
}
