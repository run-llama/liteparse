//! Resolved source direction, kept separate from visual ordering and generated label support.
use serde::Serialize;

/// One active-RTL physical body paragraph in the accepted-current property view.
/// Source text remains in logical order; visual bidi runs, mirrored glyphs and shaping are unresolved.
#[derive(Clone, Debug, Serialize)]
pub struct ParagraphDirectionEvidence {
    pub start_cp: usize,
    pub paragraph_mark_cp: usize,
    pub paragraph_direction: &'static str,
    pub text_order: &'static str,
    pub visual_order_and_shaping: &'static str,
    pub list_override_index: Option<usize>,
    pub list_level: Option<u8>,
    pub label_properties: Option<super::rtl::RtlLabelProperties>,
    pub list_label_status: &'static str,
}

impl ParagraphDirectionEvidence {
    pub(super) fn active_rtl(
        start_cp: usize,
        paragraph_mark_cp: usize,
        list: Option<(usize, u8)>,
    ) -> Self {
        Self {
            start_cp,
            paragraph_mark_cp,
            paragraph_direction: "right_to_left",
            text_order: "source_logical_order",
            visual_order_and_shaping: "unresolved",
            list_override_index: list.map(|(index, _)| index),
            list_level: list.map(|(_, level)| level),
            label_properties: None,
            list_label_status: if list.is_some() {
                "unresolved_right_to_left_label_properties"
            } else {
                "not_selected_for_numbering"
            },
        }
    }
}

/// Source-resolved row direction at a validated physical table-row mark.
/// Physical source cell ownership is retained; visual column ordering remains unresolved.
#[derive(Clone, Debug, Serialize)]
pub struct TableDirectionEvidence {
    pub paragraph_start_cp: usize,
    pub row_mark_cp: usize,
    pub tf_bidi: Option<bool>,
    pub tf_bidi90: Option<bool>,
    pub right_to_left: bool,
    pub visual_order: &'static str,
}
