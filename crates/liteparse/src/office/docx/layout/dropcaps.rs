//! Positioning of explicit drop-cap frames beside their anchor paragraph.
use super::*;
use crate::office::docx::model::{Anchored, HRel, Inline, Picture, VRel, Wrap};
use crate::office::docx::props::DropCap;

impl Flow<'_> {
    /// Pairs a single text-only cap with an adjacent ordinary body paragraph.
    /// Explicit glyph metrics are retained; the requested lines determine the
    /// exclusion height and baseline placement, independently of frame y offsets.
    pub(super) fn drop_cap_pair(
        &self,
        cap: &Block,
        following: &Block,
        width: f64,
    ) -> Option<Paragraph> {
        let (Block::Para(cap), Block::Para(following)) = (cap, following) else {
            return None;
        };
        let f = cap
            .style
            .resolved_frame
            .as_ref()
            .or(cap.style.frame.as_ref())?;
        let mode = f.drop_cap?;
        if mode == DropCap::Unsupported
            || f.drop_lines == 0
            || cap.style.bidi
            || following.style.bidi
            || following.style.frame.is_some()
            || following.style.resolved_frame.is_some()
            || cap.num.is_some()
            || cap.inlines.is_empty()
            || !cap
                .inlines
                .iter()
                .all(|i| matches!(i, Inline::Text(..) | Inline::Bookmark(_)))
        {
            return None;
        }
        let mut plain = cap.clone();
        let st = Rc::make_mut(&mut plain.style);
        st.frame = None;
        st.resolved_frame = None;
        st.left = 0.0;
        st.right = 0.0;
        st.first = 0.0;
        st.jc = crate::office::docx::styles::Jc::Left;
        let mut cap_lines = layout_paragraph(&plain, width, self.env);
        if cap_lines.len() != 1 {
            return None;
        }
        let cap_line = cap_lines.pop()?;
        let cap_width = cap_line
            .items
            .iter()
            .filter_map(|i| match i {
                Item::Text(t) => Some(t.transform.tx as f64 + t.width),
                _ => None,
            })
            .fold(0.0_f64, f64::max);
        if cap_width <= 0.0 || cap_width >= width {
            return None;
        }
        let space = if f.wrap == "around" { f.h_space } else { 0.0 };
        let left = following.style.left;
        let x = if mode == DropCap::Margin {
            left - cap_width - space
        } else {
            left + following.style.first
        };
        let wrap = if mode == DropCap::Margin || f.wrap == "none" {
            Wrap::None
        } else if f.wrap == "notBeside" {
            Wrap::TopAndBottom
        } else {
            Wrap::Square
        };
        let measure = |body: &[Line]| -> Option<(f64, f64)> {
            body.first()?;
            let n = f.drop_lines.min(body.len());
            let last = &body[n - 1];
            let leading: f64 = body[..n - 1].iter().map(|l| l.height + l.skip).sum();
            let tail = f.drop_lines.saturating_sub(n) as f64 * last.height;
            let height = leading + tail + last.skip + last.height;
            height
                .is_finite()
                .then_some((height, leading + tail + last.skip + last.baseline))
        };
        let unwrapped = layout_paragraph(following, width, self.env);
        let first_height = unwrapped.first()?.height;
        let (mut height, mut baseline) = measure(&unwrapped)?;
        if wrap == Wrap::Square {
            // Rewrapping can move a larger run into the initial lines. Recompute
            // the requested line span until its height and baseline stabilize.
            let mut stable = false;
            for _ in 0..8 {
                let wrapped = crate::office::docx::text::WrapIn {
                    contour: Vec::new(),
                    rects: vec![(0.0, height, x - space, x + cap_width + space)],
                    est_h: first_height,
                    min_gap: 18.0,
                };
                let next = measure(&crate::office::docx::text::layout_paragraph_wrapped(
                    following, width, self.env, &wrapped,
                ))?;
                stable = (next.0 - height).abs() < 0.01 && (next.1 - baseline).abs() < 0.01;
                (height, baseline) = next;
                if stable {
                    break;
                }
            }
            if !stable {
                return None;
            }
        }
        let mut text_items = cap_line.items;
        for it in &mut text_items {
            translate_item(it, 0.0, baseline - cap_line.baseline);
        }
        let anchor = Anchored {
            pic: Picture {
                part: None,
                w: cap_width,
                h: height,
                crop: (0.0, 0.0, 0.0, 0.0),
                alt: None,
                shape: None,
                effects: Vec::new(),
                alpha: 1.0,
                graphic: None,
                shadow: None,
            },
            h_rel: HRel::Column,
            x: Ok(x),
            v_rel: VRel::Line,
            y: Ok(0.0),
            behind: false,
            z: 0,
            wrap,
            contour: None,
            dist: [0.0, 0.0, space, space],
            wrap_side: "bothSides".into(),
            text_items,
            frame_children: Vec::new(),
            text_frame: false,
            turned: false,
        };
        let mut paired = following.clone();
        paired.inlines.insert(0, Inline::Anchor(Box::new(anchor)));
        Rc::make_mut(&mut paired.style).page_break_before |= cap.style.page_break_before;
        // Computing body metrics above does not consume the following paragraph.
        // Its source remains available for wrapping and pagination reflow.
        Some(paired)
    }
}

#[cfg(test)]
mod tests;
