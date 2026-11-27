//! [MS-PPT] record headers and the record types this converter reads.
//!
//! Every record starts with an 8-byte header: a u16 holding the version (low 4 bits,
//! 0xF for containers) and the instance (high 12 bits), a u16 record type and a u32
//! body length. Containers hold a sequence of child records in their body.

pub const DOCUMENT: u16 = 0x03E8;
pub const DOCUMENT_ATOM: u16 = 0x03E9;
pub const SLIDE: u16 = 0x03EE;
pub const ENVIRONMENT: u16 = 0x03F2;
pub const SLIDE_PERSIST_ATOM: u16 = 0x03F3;
pub const SLIDE_SHOW_SLIDE_INFO: u16 = 0x03F9;
pub const PP_DRAWING: u16 = 0x040C;
pub const FONT_COLLECTION: u16 = 0x07D5;
pub const OUTLINE_TEXT_REF_ATOM: u16 = 0x0F9E;
pub const TEXT_HEADER_ATOM: u16 = 0x0F9F;
pub const TEXT_CHARS_ATOM: u16 = 0x0FA0;
pub const TEXT_BYTES_ATOM: u16 = 0x0FA8;
pub const FONT_ENTITY_ATOM: u16 = 0x0FB7;
pub const SLIDE_LIST_WITH_TEXT: u16 = 0x0FF0;
pub const USER_EDIT_ATOM: u16 = 0x0FF5;
pub const CURRENT_USER_ATOM: u16 = 0x0FF6;
pub const PERSIST_DIRECTORY_ATOM: u16 = 0x1772;

pub const SLIDE_ATOM: u16 = 0x03EF;
pub const MAIN_MASTER: u16 = 0x03F8;
pub const PP_DRAWING_GROUP: u16 = 0x040B;
pub const ROUND_TRIP_THEME: u16 = 0x040E;
pub const ROUND_TRIP_COLOR_MAPPING: u16 = 0x040F;
pub const ROUND_TRIP_LAYOUT: u16 = 0x041E;
pub const ROUND_TRIP_LAYOUT_ID: u16 = 0x0422;
pub const ROUND_TRIP_TEXT_STYLES: u16 = 0x0423;
pub const COLOR_SCHEME_ATOM: u16 = 0x07F0;
pub const OE_PLACEHOLDER_ATOM: u16 = 0x0BC3;
pub const STYLE_TEXT_PROP_ATOM: u16 = 0x0FA1;
pub const TEXT_MASTER_STYLE_ATOM: u16 = 0x0FA3;
pub const TEXT_CF_EXCEPTION_ATOM: u16 = 0x0FA4;
pub const TEXT_PF_EXCEPTION_ATOM: u16 = 0x0FA5;
pub const TEXT_RULER_ATOM: u16 = 0x0FA6;
pub const CSTRING: u16 = 0x0FBA;
pub const EX_OBJ_LIST: u16 = 0x0409;
pub const EX_EMBED: u16 = 0x0FCC;
pub const EX_OLE_OBJ_ATOM: u16 = 0x0FC3;
pub const EX_OBJ_REF_ATOM: u16 = 0x0BC1;
/// ProgTagsContainer, ProgBinaryTag and its BinaryTagDataBlob (an atom holding records).
pub const PROG_TAGS: u16 = 0x1388;
pub const PROG_BINARY_TAG: u16 = 0x138A;
pub const BINARY_TAG_DATA_BLOB: u16 = 0x138B;
/// StyleTextProp9Atom: PowerPoint 2000 paragraph extensions (automatic numbering).
pub const STYLE_TEXT_PROP9_ATOM: u16 = 0x0FAC;
/// DocInfoListContainer, and the PP9 outline text extensions it can hold: entries of
/// an OutlineTextPropsHeaderExAtom (slide id, text type) and a StyleTextProp9Atom.
pub const DOC_INFO_LIST: u16 = 0x07D0;
pub const OUTLINE_TEXT_PROPS9: u16 = 0x0FAE;
pub const OUTLINE_TEXT_PROPS_HEADER: u16 = 0x0FAF;
pub const SLIDE_NUMBER_MC_ATOM: u16 = 0x0FD8;
pub const TEXT_INTERACTIVE_INFO_ATOM: u16 = 0x0FDF;
pub const HEADERS_FOOTERS: u16 = 0x0FD9;
pub const HEADERS_FOOTERS_ATOM: u16 = 0x0FDA;
pub const DATE_TIME_MC_ATOM: u16 = 0x0FF7;
pub const GENERIC_DATE_MC_ATOM: u16 = 0x0FF8;
pub const HEADER_MC_ATOM: u16 = 0x0FF9;
pub const FOOTER_MC_ATOM: u16 = 0x0FFA;
pub const RTF_DATE_TIME_MC_ATOM: u16 = 0x1015;

pub const DGG_CONTAINER: u16 = 0xF000;
pub const DG_CONTAINER: u16 = 0xF002;
pub const SPGR_CONTAINER: u16 = 0xF003;
pub const SP_CONTAINER: u16 = 0xF004;
pub const FSPGR: u16 = 0xF009;
pub const FSP: u16 = 0xF00A;
pub const OPT: u16 = 0xF00B;
pub const SECONDARY_OPT: u16 = 0xF121;
pub const TERTIARY_OPT: u16 = 0xF122;
pub const CLIENT_TEXTBOX: u16 = 0xF00D;
pub const CHILD_ANCHOR: u16 = 0xF00F;
pub const CLIENT_ANCHOR: u16 = 0xF010;
pub const CLIENT_DATA: u16 = 0xF011;

/// One record: its type, instance, and body as a byte range of the stream.
#[derive(Clone, Copy, Debug)]
pub struct Rec {
    pub inst: u16,
    pub kind: u16,
    pub start: usize,
    pub end: usize,
}

impl Rec {
    pub fn len(&self) -> usize {
        self.end - self.start
    }
}

/// The record at `off`, or `None` when its header or body runs past `data`.
pub fn at(data: &[u8], off: usize) -> Option<Rec> {
    let vi = u16_at(data, off)?;
    let kind = u16_at(data, off + 2)?;
    let len = u32_at(data, off + 4)? as usize;
    let start = off.checked_add(8)?;
    let end = start.checked_add(len)?;
    (end <= data.len()).then_some(Rec {
        inst: vi >> 4,
        kind,
        start,
        end,
    })
}

/// Child records of a container, in order. A child that overruns its parent ends the
/// list, so a damaged container yields the children before the damage.
pub fn children(data: &[u8], parent: Rec) -> Vec<Rec> {
    let bounded = &data[..parent.end];
    let mut out = Vec::new();
    let mut off = parent.start;
    while off + 8 <= parent.end {
        let Some(r) = at(bounded, off) else { break };
        off = r.end;
        out.push(r);
    }
    out
}

/// The first child of `parent` with record type `kind`.
pub fn child(data: &[u8], parent: Rec, kind: u16) -> Option<Rec> {
    children(data, parent).into_iter().find(|r| r.kind == kind)
}

pub fn u16_at(data: &[u8], off: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        data.get(off..off.checked_add(2)?)?.try_into().ok()?,
    ))
}

pub fn u32_at(data: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        data.get(off..off.checked_add(4)?)?.try_into().ok()?,
    ))
}

pub fn i32_at(data: &[u8], off: usize) -> Option<i32> {
    u32_at(data, off).map(|v| v as i32)
}

pub fn i16_at(data: &[u8], off: usize) -> Option<i16> {
    u16_at(data, off).map(|v| v as i16)
}
