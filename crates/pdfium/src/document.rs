use crate::error::PdfiumError;
use crate::ffi;
use crate::library::Library;
use crate::page::Page;

/// An open PDF document.
///
/// The `'lib` lifetime ties this `Document` to the [`Library`] that opened
/// it, statically guaranteeing that no PDFium calls happen after the
/// process-wide PDFium lock has been released.
pub struct Document<'lib> {
    pub(crate) handle: pdfium_sys::FPDF_DOCUMENT,
    /// Per-page `/UserUnit` multipliers (see [`crate::user_unit`]); empty
    /// when no page declares one, which is the overwhelmingly common case.
    /// PDFium ignores `/UserUnit`, so pages carry this multiplier and apply
    /// it to all viewport-space geometry and rendering.
    pub(crate) page_user_units: Vec<f32>,
    pub(crate) _lib: std::marker::PhantomData<&'lib Library>,
}

/// PDFium's form-fill environment for an open document. The callback table
/// must remain alive until the handle is closed, even though LiteParse leaves
/// every callback null and uses the environment for read-only field access.
pub struct FormEnvironment<'doc, 'lib: 'doc> {
    pub(crate) handle: pdfium_sys::FPDF_FORMHANDLE,
    _callbacks: Box<pdfium_sys::FPDF_FORMFILLINFO>,
    _doc: std::marker::PhantomData<&'doc Document<'lib>>,
}

/// One entry in the document's outline (bookmarks tree).
#[derive(Debug, Clone)]
pub struct OutlineEntry {
    /// Hierarchy depth, 1-based (top-level entries are level 1).
    pub level: u8,
    /// Bookmark title.
    pub title: String,
    /// Zero-based page index of the destination, or `None` if the destination
    /// isn't a page in this document (external link, missing dest, etc).
    pub page_index: Option<i32>,
    /// Y coordinate of the destination on the page in PDF user space (origin
    /// bottom-left), or `None` if the destination doesn't specify one. To
    /// compare against viewport-space line bboxes (origin top-left) use
    /// `page_height - y`.
    pub y: Option<f32>,
}

/// One raw packet from an XFA form document's `/XFA` array.
#[derive(Debug, Clone)]
pub struct XfaPacket {
    /// Zero-based index in the XFA array.
    pub index: i32,
    /// Packet name (e.g. `template`, `datasets`), when present.
    pub name: Option<String>,
    /// Raw packet bytes (usually XML), when readable.
    pub content: Option<Vec<u8>>,
}

/// Signature summary used for document provenance metadata.
#[derive(Debug, Clone, Copy, Default)]
pub struct SignatureSummary {
    /// `None` when the loaded pdfium build has no signature API, which is not
    /// the same as a document with zero signatures.
    pub count: Option<u32>,
    /// `None` when signatures exist but PDFium did not expose any byte range.
    pub byte_range_reaches_eof: Option<bool>,
}

/// The `fpdf_signature` entry points, resolved together. `None` when the
/// loaded pdfium build does not export them.
struct SignatureApi {
    count: unsafe extern "C" fn(pdfium_sys::FPDF_DOCUMENT) -> std::os::raw::c_int,
    object: unsafe extern "C" fn(
        pdfium_sys::FPDF_DOCUMENT,
        std::os::raw::c_int,
    ) -> pdfium_sys::FPDF_SIGNATURE,
    byte_range: unsafe extern "C" fn(
        pdfium_sys::FPDF_SIGNATURE,
        *mut std::os::raw::c_int,
        std::os::raw::c_ulong,
    ) -> std::os::raw::c_ulong,
}

impl SignatureApi {
    #[cfg(not(target_arch = "wasm32"))]
    fn load() -> Option<Self> {
        let bindings = pdfium_sys::dynamic::pdfium();
        Some(Self {
            count: bindings.FPDF_GetSignatureCount?,
            object: bindings.FPDF_GetSignatureObject?,
            byte_range: bindings.FPDFSignatureObj_GetByteRange?,
        })
    }

    #[cfg(target_arch = "wasm32")]
    fn load() -> Option<Self> {
        Some(Self {
            count: pdfium_sys::FPDF_GetSignatureCount,
            object: pdfium_sys::FPDF_GetSignatureObject,
            byte_range: pdfium_sys::FPDFSignatureObj_GetByteRange,
        })
    }
}

/// The `/Subtype` an overlay copy gives the widgets it keeps: no annotation
/// type PDFium knows, so it generates no appearance and attaches no popup.
const RETYPED_WIDGET: &str = "LiteParseOverlay";

/// The page-editing entry points [`Document::widget_appearance_copy`] and
/// [`Document::annotation_overlay_copy`] need, resolved together so a build
/// missing any of them degrades to "no widget text" or "no overlay" rather
/// than failing the whole pdfium load. Only the overlay retypes widgets, so
/// `set_string` alone is optional: without it the widget text still works.
struct PageEditApi {
    create: unsafe extern "C" fn() -> pdfium_sys::FPDF_DOCUMENT,
    import: unsafe extern "C" fn(
        pdfium_sys::FPDF_DOCUMENT,
        pdfium_sys::FPDF_DOCUMENT,
        *const std::os::raw::c_int,
        std::os::raw::c_ulong,
        std::os::raw::c_int,
    ) -> pdfium_sys::FPDF_BOOL,
    remove_object: unsafe extern "C" fn(
        pdfium_sys::FPDF_PAGE,
        pdfium_sys::FPDF_PAGEOBJECT,
    ) -> pdfium_sys::FPDF_BOOL,
    destroy_object: unsafe extern "C" fn(pdfium_sys::FPDF_PAGEOBJECT),
    generate_content: unsafe extern "C" fn(pdfium_sys::FPDF_PAGE) -> pdfium_sys::FPDF_BOOL,
    flatten:
        unsafe extern "C" fn(pdfium_sys::FPDF_PAGE, std::os::raw::c_int) -> std::os::raw::c_int,
    set_flags: unsafe extern "C" fn(
        pdfium_sys::FPDF_ANNOTATION,
        std::os::raw::c_int,
    ) -> pdfium_sys::FPDF_BOOL,
    set_string: Option<
        unsafe extern "C" fn(
            pdfium_sys::FPDF_ANNOTATION,
            pdfium_sys::FPDF_BYTESTRING,
            pdfium_sys::FPDF_WIDESTRING,
        ) -> pdfium_sys::FPDF_BOOL,
    >,
}

impl PageEditApi {
    #[cfg(not(target_arch = "wasm32"))]
    fn load() -> Option<Self> {
        let bindings = pdfium_sys::dynamic::pdfium();
        Some(Self {
            create: bindings.FPDF_CreateNewDocument?,
            import: bindings.FPDF_ImportPagesByIndex?,
            remove_object: bindings.FPDFPage_RemoveObject?,
            destroy_object: bindings.FPDFPageObj_Destroy?,
            generate_content: bindings.FPDFPage_GenerateContent?,
            flatten: bindings.FPDFPage_Flatten?,
            set_flags: bindings.FPDFAnnot_SetFlags?,
            set_string: bindings.FPDFAnnot_SetStringValue,
        })
    }

    #[cfg(target_arch = "wasm32")]
    fn load() -> Option<Self> {
        Some(Self {
            create: pdfium_sys::FPDF_CreateNewDocument,
            import: pdfium_sys::FPDF_ImportPagesByIndex,
            remove_object: pdfium_sys::FPDFPage_RemoveObject,
            destroy_object: pdfium_sys::FPDFPageObj_Destroy,
            generate_content: pdfium_sys::FPDFPage_GenerateContent,
            flatten: pdfium_sys::FPDFPage_Flatten,
            set_flags: pdfium_sys::FPDFAnnot_SetFlags,
            set_string: Some(pdfium_sys::FPDFAnnot_SetStringValue),
        })
    }
}

impl<'lib> Document<'lib> {
    pub fn page_count(&self) -> i32 {
        unsafe { ffi!(FPDF_GetPageCount(self.handle)) }
    }

    pub fn form_type(&self) -> i32 {
        unsafe { ffi!(FPDF_GetFormType(self.handle)) }
    }

    /// Whether the catalog declares the document tagged (`/MarkInfo /Marked true`).
    /// A structure tree can be present without this flag — residual or stale
    /// tagging — so callers that treat the tree as authoritative gate on it.
    pub fn is_tagged(&self) -> bool {
        (unsafe { ffi!(FPDFCatalog_IsTagged(self.handle)) }) != 0
    }

    /// Initialize read-only AcroForm access. Returns `None` for documents with
    /// no form catalog or when PDFium rejects the form-fill environment.
    pub fn form_environment(&self) -> Option<FormEnvironment<'_, 'lib>> {
        if self.form_type() == 0 {
            return None;
        }
        self.appearance_environment()
    }

    /// A form-fill environment even for a document without a form. Loading a
    /// page into one (see [`Page::load_form_widgets`]) is what makes PDFium
    /// write the appearance streams it generates — for widgets saved without
    /// one, and for Square, Ink and other markup saved without `/AP` — before
    /// anything is rendered. `None` if PDFium cannot create it.
    pub fn appearance_environment(&self) -> Option<FormEnvironment<'_, 'lib>> {
        let mut callbacks = Box::new(pdfium_sys::FPDF_FORMFILLINFO::default());
        callbacks.version = 1;
        let handle = unsafe {
            ffi!(FPDFDOC_InitFormFillEnvironment(
                self.handle,
                &mut *callbacks
            ))
        };
        (!handle.is_null()).then_some(FormEnvironment {
            handle,
            _callbacks: callbacks,
            _doc: std::marker::PhantomData,
        })
    }

    pub fn page(&self, index: i32) -> Result<Page<'_, 'lib>, PdfiumError> {
        let handle = unsafe { ffi!(FPDF_LoadPage(self.handle, index)) };
        if handle.is_null() {
            return Err(PdfiumError::PageNotFound);
        }
        // Prefer the fork's dict-reading API; the byte-scan table (see
        // `crate::user_unit`) is the fallback for binaries that predate it.
        let user_unit = Self::user_unit_from_api(handle).unwrap_or_else(|| {
            self.page_user_units
                .get(index as usize)
                .copied()
                .unwrap_or(1.0)
        });
        Ok(Page {
            handle,
            doc_handle: self.handle,
            user_unit,
            _doc: std::marker::PhantomData,
        })
    }

    /// Read `/UserUnit` through the fork's `FPDFPage_GetUserUnit` export.
    /// `None` when the loaded pdfium binary does not provide it.
    #[cfg(not(target_arch = "wasm32"))]
    fn user_unit_from_api(page: pdfium_sys::FPDF_PAGE) -> Option<f32> {
        let get_user_unit = pdfium_sys::dynamic::pdfium().FPDFPage_GetUserUnit?;
        let user_unit = unsafe { get_user_unit(page) };
        // The API already clamps to >= 1.0; guard anyway so a misbehaving
        // binary can't zero out all geometry.
        Some(if user_unit.is_finite() && user_unit >= 1.0 {
            user_unit
        } else {
            1.0
        })
    }

    /// On wasm the export is statically linked (the pinned pdfium-binaries
    /// release ships it), so unlike the dynamic path this can never be
    /// absent at runtime — bumping the pin below a release that carries
    /// `FPDFPage_GetUserUnit` would be a link error, not a silent fallback.
    #[cfg(target_arch = "wasm32")]
    fn user_unit_from_api(page: pdfium_sys::FPDF_PAGE) -> Option<f32> {
        let user_unit = unsafe { pdfium_sys::FPDFPage_GetUserUnit(page) };
        Some(if user_unit.is_finite() && user_unit >= 1.0 {
            user_unit
        } else {
            1.0
        })
    }

    /// A private one-page document whose page 0 is page `index` reduced to its
    /// visible form-widget appearances, flattened into page content so pdfium's
    /// text API reports the glyphs they paint.
    ///
    /// The page is imported into a fresh document, every annotation that is not a
    /// visible widget is hidden, the original content objects are removed, and the
    /// page is flattened. Extracting the appearances on a copy — rather than
    /// flattening them over the ordinary text — keeps pdfium from suppressing
    /// either run when their origins coincide; this document's own pages are never
    /// touched. `None` when the loaded pdfium build lacks the editing API or any
    /// step fails; the caller keeps the page text it already has. Load page 0 of
    /// the result fresh: a page handle open across a flatten keeps its old text.
    pub fn widget_appearance_copy(&self, index: i32) -> Option<Document<'lib>> {
        self.annotation_only_copy(index, true, |_, is_widget, visible| is_widget && visible)
    }

    /// A private one-page document whose page 0 holds only the visible
    /// annotations at `annot_indices` on page `index`, over an empty page.
    ///
    /// `FPDF_RenderPageBitmap` never paints widget annotations — only the
    /// form-fill layer does, and only for widgets its field tree knows. Here
    /// the kept widgets are retyped so the ordinary annotation render paints
    /// their appearance like any other annotation, so rendering page 0 with
    /// `FPDF_ANNOT` over a raster of the same page draws them in `/Annots`
    /// order, together with any annotations stacked above them (see
    /// [`Page::annotations_above`]), with PDFium's own placement: `/NoRotate`
    /// compensation, QuadPoints-based markup, appearance matrices. Every other
    /// annotation is hidden. `None` under the same conditions as
    /// [`Self::widget_appearance_copy`].
    pub fn annotation_overlay_copy(
        &self,
        index: i32,
        annot_indices: &[i32],
    ) -> Option<Document<'lib>> {
        self.annotation_only_copy(index, false, |annot_index, _, visible| {
            visible && annot_indices.contains(&annot_index)
        })
    }

    /// Import page `index` into a fresh document, hide every annotation for
    /// which `keep(annot_index, is_widget, is_visible)` is false, and drop the
    /// page content. With `flatten`, flatten what remains into page content;
    /// otherwise retype the kept widgets so the annotation render paints them.
    fn annotation_only_copy(
        &self,
        index: i32,
        flatten: bool,
        keep: impl Fn(i32, bool, bool) -> bool,
    ) -> Option<Document<'lib>> {
        let api = PageEditApi::load()?;
        let set_string = if flatten { None } else { Some(api.set_string?) };
        let handle = unsafe { (api.create)() };
        if handle.is_null() {
            return None;
        }
        let copy = Document {
            handle,
            page_user_units: self
                .page_user_units
                .get(index as usize)
                .map(|unit| vec![*unit])
                .unwrap_or_default(),
            _lib: std::marker::PhantomData,
        };
        if unsafe { (api.import)(copy.handle, self.handle, &index, 1, 0) } == 0 {
            return None;
        }
        let page = copy.page(0).ok()?;
        let retyped: Vec<u16> = RETYPED_WIDGET
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let annot_count = unsafe { ffi!(FPDFPage_GetAnnotCount(page.handle)) };
        for annot_index in 0..annot_count {
            let annot = unsafe { ffi!(FPDFPage_GetAnnot(page.handle, annot_index)) };
            if annot.is_null() {
                return None;
            }
            let flags = unsafe { ffi!(FPDFAnnot_GetFlags(annot)) };
            // Each mode keeps what PDFium itself would draw: flattening for
            // display also drops Invisible annotations, the render does not.
            let suppressed = if flatten {
                crate::page::SCREEN_HIDDEN | pdfium_sys::FPDF_ANNOT_FLAG_INVISIBLE as i32
            } else {
                crate::page::SCREEN_HIDDEN
            };
            let is_widget = unsafe { ffi!(FPDFAnnot_GetSubtype(annot)) }
                == pdfium_sys::FPDF_ANNOT_WIDGET as i32;
            let ok = if !keep(annot_index, is_widget, flags & suppressed == 0) {
                let hidden = flags | pdfium_sys::FPDF_ANNOT_FLAG_HIDDEN as i32;
                unsafe { (api.set_flags)(annot, hidden) != 0 }
            } else if let Some(set_string) = set_string.filter(|_| is_widget) {
                // `CPDF_AnnotList` skips widgets unless the form layer draws;
                // under any other subtype their appearance draws as is.
                unsafe { set_string(annot, c"Subtype".as_ptr(), retyped.as_ptr()) != 0 }
            } else {
                true
            };
            unsafe { ffi!(FPDFPage_CloseAnnot(annot)) };
            if !ok {
                return None;
            }
        }
        for object_index in (0..unsafe { ffi!(FPDFPage_CountObjects(page.handle)) }).rev() {
            let object = unsafe { ffi!(FPDFPage_GetObject(page.handle, object_index)) };
            if object.is_null() || unsafe { (api.remove_object)(page.handle, object) } == 0 {
                return None;
            }
            unsafe { (api.destroy_object)(object) };
        }
        if unsafe { (api.generate_content)(page.handle) } == 0
            || flatten
                && unsafe { (api.flatten)(page.handle, pdfium_sys::FLAT_NORMALDISPLAY as i32) }
                    != pdfium_sys::FLATTEN_SUCCESS as i32
        {
            return None;
        }
        drop(page);
        Some(copy)
    }

    /// Flatten the visible form-widget appearances on `index` into the page
    /// content stream and hand back a freshly loaded page reflecting them.
    ///
    /// Flattening mutates this document in place and invalidates the page
    /// handle it ran on, so the load/flatten/reload sequence lives here rather
    /// than at call sites where a stale handle would be easy to keep using.
    /// Returns `Ok(None)` when nothing was flattened — the caller should keep
    /// using its existing page.
    pub fn flatten_form_widgets(&self, index: i32) -> Result<Option<Page<'_, 'lib>>, PdfiumError> {
        {
            let page = self.page(index)?;
            if !page.flatten_form_widgets_for_display() {
                return Ok(None);
            }
        }
        self.page(index).map(Some)
    }

    /// Value of `tag` in the document's Info dictionary (`Creator`,
    /// `Producer`, `CreationDate`, ...). `None` when the document has no Info
    /// dictionary; `Some("")` when it has one but the key is missing or
    /// empty — pdfium reports both the same way.
    pub fn meta_text(&self, tag: &str) -> Option<String> {
        let tag_c = std::ffi::CString::new(tag).ok()?;
        let needed = unsafe {
            ffi!(FPDF_GetMetaText(
                self.handle,
                tag_c.as_ptr(),
                std::ptr::null_mut(),
                0
            ))
        } as usize;
        if needed < 2 {
            return None;
        }
        // `needed` is byte length of the UTF-16 value including a trailing NUL.
        let mut buf: Vec<u16> = vec![0; needed / 2];
        let written = unsafe {
            ffi!(FPDF_GetMetaText(
                self.handle,
                tag_c.as_ptr(),
                buf.as_mut_ptr() as *mut std::os::raw::c_void,
                needed as std::os::raw::c_ulong,
            ))
        } as usize;
        if written < 2 {
            return None;
        }
        let chars = written / 2;
        let end = if buf.get(chars - 1) == Some(&0) {
            chars - 1
        } else {
            chars
        };
        // `end == 0` is an Info dictionary that exists but has no (or an
        // empty) value for `tag`; pdfium cannot tell those two apart. `None`
        // is reserved for a document with no Info dictionary at all, which
        // callers reporting provenance need to distinguish from `""`.
        Some(String::from_utf16_lossy(&buf[..end]))
    }

    /// The document's `/PageLabels` entry for a zero-based page index, when
    /// the document defines one.
    ///
    /// This is the label a reader displays for the page — `"iv"`, `"A-1"`,
    /// `"12"` — which is not always the page's position in the document.
    /// `None` when the document has no `/PageLabels` tree, or none covering
    /// this page; callers should fall back to the one-based page number.
    pub fn page_label(&self, page_index: i32) -> Option<String> {
        if page_index < 0 {
            return None;
        }
        let needed = unsafe {
            ffi!(FPDF_GetPageLabel(
                self.handle,
                page_index,
                std::ptr::null_mut(),
                0
            ))
        } as usize;
        // `needed` is the byte length of the UTF-16 label including its
        // trailing NUL, so anything under 4 bytes is an empty or absent label.
        if needed < 4 {
            return None;
        }
        let mut buf: Vec<u16> = vec![0; needed / 2];
        let written = unsafe {
            ffi!(FPDF_GetPageLabel(
                self.handle,
                page_index,
                buf.as_mut_ptr() as *mut std::os::raw::c_void,
                needed as std::os::raw::c_ulong,
            ))
        } as usize;
        if written < 4 {
            return None;
        }
        let chars = written / 2;
        let end = if buf.get(chars - 1) == Some(&0) {
            chars - 1
        } else {
            chars
        };
        let label = String::from_utf16_lossy(&buf[..end]);
        (!label.is_empty()).then_some(label)
    }

    /// Encoded PDF version (`14` means PDF 1.4), when present.
    pub fn file_version(&self) -> Option<i32> {
        let mut version = 0;
        let ok = unsafe { ffi!(FPDF_GetFileVersion(self.handle, &mut version)) };
        (ok != 0).then_some(version)
    }

    /// PDF security-handler revision, or `-1` for an unencrypted document.
    pub fn security_handler_revision(&self) -> i32 {
        unsafe { ffi!(FPDF_GetSecurityHandlerRevision(self.handle)) }
    }

    /// Document permission flags reported by PDFium.
    pub fn permissions(&self) -> u64 {
        unsafe { ffi!(FPDF_GetDocPermissions(self.handle)) as u64 }
    }

    /// Count signatures and determine whether every readable final byte-range
    /// segment reaches the current end of the file. Needs `file_size` to answer
    /// the byte-range question at all — without it the verdict stays `None`
    /// rather than defaulting to "reaches EOF".
    pub fn signature_summary(&self, file_size: Option<u64>) -> SignatureSummary {
        const MAX_BYTE_RANGE_VALUES: usize = 8;
        let Some(api) = SignatureApi::load() else {
            return SignatureSummary::default();
        };
        let count = unsafe { (api.count)(self.handle) }.max(0) as u32;
        let Some(file_size) = file_size.filter(|_| count > 0) else {
            return SignatureSummary {
                count: Some(count),
                byte_range_reaches_eof: None,
            };
        };

        let mut known = false;
        let mut reaches_eof = true;
        for index in 0..count {
            let signature = unsafe { (api.object)(self.handle, index as i32) };
            if signature.is_null() {
                continue;
            }
            let mut ranges = [0i32; MAX_BYTE_RANGE_VALUES];
            let len = unsafe {
                (api.byte_range)(
                    signature,
                    ranges.as_mut_ptr(),
                    ranges.len() as std::os::raw::c_ulong,
                )
            } as usize;
            if !(2..=MAX_BYTE_RANGE_VALUES).contains(&len) {
                continue;
            }
            known = true;
            let start = i64::from(ranges[len - 2]);
            let length = i64::from(ranges[len - 1]);
            if start < 0
                || length < 0
                || u64::try_from(start + length)
                    .ok()
                    .is_some_and(|range_end| range_end < file_size)
            {
                reaches_eof = false;
            }
        }
        SignatureSummary {
            count: Some(count),
            byte_range_reaches_eof: known.then_some(reaches_eof),
        }
    }

    /// Number of packets in the document's `/XFA` array (0 for non-XFA docs).
    pub fn xfa_packet_count(&self) -> i32 {
        unsafe { ffi!(FPDF_GetXFAPacketCount(self.handle)) }
    }

    /// Read every packet from the document's `/XFA` array. Empty for
    /// non-XFA documents. Individual name/content read failures surface as
    /// `None` fields rather than dropping the packet.
    pub fn xfa_packets(&self) -> Vec<XfaPacket> {
        let count = self.xfa_packet_count();
        if count <= 0 {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(count as usize);
        for index in 0..count {
            let name_len = unsafe {
                ffi!(FPDF_GetXFAPacketName(
                    self.handle,
                    index,
                    std::ptr::null_mut(),
                    0
                ))
            } as usize;
            let name = (name_len > 0)
                .then(|| {
                    let mut buf = vec![0u8; name_len];
                    let written = unsafe {
                        ffi!(FPDF_GetXFAPacketName(
                            self.handle,
                            index,
                            buf.as_mut_ptr() as *mut std::os::raw::c_void,
                            name_len as std::os::raw::c_ulong,
                        ))
                    } as usize;
                    if written == 0 {
                        return None;
                    }
                    buf.truncate(written.min(name_len));
                    while buf.last() == Some(&0) {
                        buf.pop();
                    }
                    Some(String::from_utf8_lossy(&buf).into_owned())
                })
                .flatten();

            let mut content_len: std::os::raw::c_ulong = 0;
            let sized = unsafe {
                ffi!(FPDF_GetXFAPacketContent(
                    self.handle,
                    index,
                    std::ptr::null_mut(),
                    0,
                    &mut content_len,
                ))
            };
            let content = (sized != 0 && content_len > 0)
                .then(|| {
                    let mut buf = vec![0u8; content_len as usize];
                    let mut written: std::os::raw::c_ulong = 0;
                    let ok = unsafe {
                        ffi!(FPDF_GetXFAPacketContent(
                            self.handle,
                            index,
                            buf.as_mut_ptr() as *mut std::os::raw::c_void,
                            content_len,
                            &mut written,
                        ))
                    };
                    if ok == 0 {
                        return None;
                    }
                    buf.truncate((written as usize).min(buf.len()));
                    Some(buf)
                })
                .flatten();

            out.push(XfaPacket {
                index,
                name,
                content,
            });
        }
        out
    }

    /// Walk the document outline (bookmarks). Returns entries in pre-order
    /// (depth-first), so parents precede their children. Empty when the
    /// document has no outline.
    pub fn outline(&self) -> Vec<OutlineEntry> {
        let mut out = Vec::new();
        let root = unsafe {
            ffi!(FPDFBookmark_GetFirstChild(
                self.handle,
                std::ptr::null_mut()
            ))
        };
        if !root.is_null() {
            self.walk_bookmark(root, 1, &mut out);
        }
        out
    }

    fn walk_bookmark(
        &self,
        bookmark: pdfium_sys::FPDF_BOOKMARK,
        level: u8,
        out: &mut Vec<OutlineEntry>,
    ) {
        let mut cur = bookmark;
        while !cur.is_null() {
            let title = read_bookmark_title(cur);
            let (page_index, y) = resolve_dest(self.handle, cur);
            out.push(OutlineEntry {
                level,
                title,
                page_index,
                y,
            });

            let child = unsafe { ffi!(FPDFBookmark_GetFirstChild(self.handle, cur)) };
            if !child.is_null() {
                self.walk_bookmark(child, level.saturating_add(1), out);
            }

            cur = unsafe { ffi!(FPDFBookmark_GetNextSibling(self.handle, cur)) };
        }
    }
}

impl FormEnvironment<'_, '_> {
    /// Execute document-level JavaScript and open actions. Some AcroForms
    /// only compute field values/appearances in these actions, so run this
    /// once after init when the environment is used for rendering. Mirrors
    /// the LlamaParse extract binary's document setup.
    pub fn run_document_actions(&self) {
        unsafe { ffi!(FORM_DoDocumentJSAction(self.handle)) };
        unsafe { ffi!(FORM_DoDocumentOpenAction(self.handle)) };
    }
}

impl Drop for FormEnvironment<'_, '_> {
    fn drop(&mut self) {
        unsafe { ffi!(FPDFDOC_ExitFormFillEnvironment(self.handle)) };
    }
}

fn read_bookmark_title(bookmark: pdfium_sys::FPDF_BOOKMARK) -> String {
    let needed = unsafe { ffi!(FPDFBookmark_GetTitle(bookmark, std::ptr::null_mut(), 0)) } as usize;
    if needed < 2 {
        return String::new();
    }
    // `needed` is byte length including a trailing UTF-16 NUL terminator.
    let mut buf: Vec<u16> = vec![0; needed / 2];
    let written = unsafe {
        ffi!(FPDFBookmark_GetTitle(
            bookmark,
            buf.as_mut_ptr() as *mut std::os::raw::c_void,
            needed as std::os::raw::c_ulong,
        ))
    } as usize;
    if written < 2 {
        return String::new();
    }
    let chars = written / 2;
    let end = if buf.get(chars - 1) == Some(&0) {
        chars - 1
    } else {
        chars
    };
    String::from_utf16_lossy(&buf[..end])
}

fn resolve_dest(
    doc: pdfium_sys::FPDF_DOCUMENT,
    bookmark: pdfium_sys::FPDF_BOOKMARK,
) -> (Option<i32>, Option<f32>) {
    let mut dest = unsafe { ffi!(FPDFBookmark_GetDest(doc, bookmark)) };
    if dest.is_null() {
        let action = unsafe { ffi!(FPDFBookmark_GetAction(bookmark)) };
        if !action.is_null() {
            dest = unsafe { ffi!(FPDFAction_GetDest(doc, action)) };
        }
    }
    if dest.is_null() {
        return (None, None);
    }
    let page_index = unsafe { ffi!(FPDFDest_GetDestPageIndex(doc, dest)) };
    let page_index = if page_index >= 0 {
        Some(page_index)
    } else {
        None
    };

    let mut has_x: pdfium_sys::FPDF_BOOL = 0;
    let mut has_y: pdfium_sys::FPDF_BOOL = 0;
    let mut has_z: pdfium_sys::FPDF_BOOL = 0;
    let mut x: f32 = 0.0;
    let mut y: f32 = 0.0;
    let mut z: f32 = 0.0;
    let ok = unsafe {
        ffi!(FPDFDest_GetLocationInPage(
            dest, &mut has_x, &mut has_y, &mut has_z, &mut x, &mut y, &mut z
        ))
    };
    let y_out = if ok != 0 && has_y != 0 { Some(y) } else { None };
    (page_index, y_out)
}

impl Drop for Document<'_> {
    fn drop(&mut self) {
        unsafe { ffi!(FPDF_CloseDocument(self.handle)) };
    }
}
