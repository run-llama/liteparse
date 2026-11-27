//! Header source slots retain guard and inactive separator text independently of pages.
use super::*;

#[derive(Debug, Serialize)]
pub struct HeaderDocument {
    pub source_text: String,
    pub ownership: crate::office::doc::headers::HeaderInspection,
    pub slots: Vec<HeaderSlot>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub inactive_unformatted_slots: Vec<InactiveSlot>,
}

#[derive(Debug, Serialize)]
pub struct InactiveSlot {
    pub index: usize,
    pub source_cp: [usize; 2],
    pub source_text: String,
    pub uncached_fields: Vec<[usize; 2]>,
}

#[derive(Debug, Serialize)]
pub struct HeaderSlot {
    pub index: usize,
    pub paragraphs: Vec<PlainParagraph>,
}

pub(super) fn prepare(
    inspection: crate::office::doc::headers::HeaderInspection,
    source: &crate::office::doc::Inspection,
) -> Result<Option<HeaderDocument>> {
    let story = &source.stories[2];
    if story.utf16_units == 0 {
        return Ok(None);
    }
    if !inspection.diagnostics.is_empty()
        || inspection.different_even_and_odd_pages.is_none()
        || inspection.sections.iter().any(|s| {
            s.different_first_page.is_none()
                || !s.unresolved.is_empty()
                || s.references
                    .iter()
                    .any(|r| !matches!(r.status, "local" | "inherited" | "absent"))
        })
        || inspection.stories.iter().any(|s| !s.unresolved.is_empty())
    {
        return Err(Error::Unsupported("render header ownership"));
    }
    let slots = inspection
        .stories
        .iter()
        .filter(|s| s.index >= 6 && s.content.is_some())
        .map(|s| HeaderSlot {
            index: s.index,
            paragraphs: Vec::new(),
        })
        .collect();
    Ok(Some(HeaderDocument {
        inactive_unformatted_slots: Vec::new(),
        source_text: story.text.clone(),
        ownership: inspection,
        slots,
    }))
}

impl HeaderDocument {
    fn inactive(&self, index: usize) -> bool {
        matches!(index, 6 | 8) && self.ownership.different_even_and_odd_pages == Some(false)
            || matches!(index, 10 | 11)
                && self.ownership.sections.len() == 1
                && self.ownership.sections[0].different_first_page == Some(false)
    }
    pub(super) fn retain_inactive_uncached(
        &mut self,
        fields: &crate::office::doc::field_inspection::FieldInspection,
    ) -> Result<()> {
        let Some(story) = fields.stories.iter().find(|s| s.kind == "headers") else {
            return Ok(());
        };
        let Some(table) = &story.table else {
            return Ok(());
        };
        if !table.topology_valid
            || !table.diagnostics.is_empty()
            || table.unindexed_marker_units != 0
        {
            return Ok(());
        }
        if table.ranges.len() > 4096 {
            return Err(Error::Limit("inactive header field count"));
        }
        let raw: Vec<_> = self.source_text.encode_utf16().collect();
        for slot in &self.slots {
            if !self.inactive(slot.index) {
                continue;
            }
            let content = self.ownership.stories[slot.index]
                .content
                .ok_or(Error::Invalid("inactive header content"))?;
            let fields: Vec<_> = table
                .ranges
                .iter()
                .filter(|f| {
                    f.result.is_none()
                        && content.start_cp <= f.source.start
                        && f.source.end <= content.end_cp
                        && table
                            .events
                            .get(table.events.partition_point(|e| e.cp < f.source.start))
                            .is_some_and(|e| {
                                e.cp == f.source.start
                                    && e.kind == crate::office::doc::fields::FieldCharacter::Begin
                                    && matches!(e.metadata, 26 | 33)
                            })
                })
                .map(|f| [f.source.start, f.source.end])
                .collect();
            if fields.is_empty() {
                continue;
            }
            let a = content
                .start_cp
                .checked_sub(self.ownership.source.start_cp)
                .ok_or(Error::Invalid("inactive header start"))?;
            let b = content
                .end_cp
                .checked_sub(self.ownership.source.start_cp)
                .ok_or(Error::Invalid("inactive header end"))?;
            let text = String::from_utf16(
                raw.get(a..b)
                    .ok_or(Error::Invalid("inactive header range"))?,
            )
            .map_err(|_| Error::Invalid("inactive header UTF16"))?;
            self.inactive_unformatted_slots.push(InactiveSlot {
                index: slot.index,
                source_cp: [content.start_cp, content.end_cp],
                source_text: text,
                uncached_fields: fields,
            });
        }
        self.slots.retain(|s| {
            !self
                .inactive_unformatted_slots
                .iter()
                .any(|i| i.index == s.index)
        });
        Ok(())
    }
    pub(super) fn inactive_ranges(&self) -> Vec<[usize; 2]> {
        self.inactive_unformatted_slots
            .iter()
            .map(|s| s.source_cp)
            .collect()
    }

    pub(super) fn validate_activation(&self, even: bool, first: bool) -> Result<()> {
        let o = &self.ownership;
        if o.dop_first_byte.map(|byte| byte & 1 != 0) != Some(even)
            || o.different_even_and_odd_pages != Some(even)
            || o.sections.is_empty()
            || o.sections[0].different_first_page != Some(first)
        {
            return Err(Error::Invalid("render header activation evidence"));
        }
        Ok(())
    }

    pub(super) fn ranges(&self) -> Result<Vec<[usize; 2]>> {
        self.slots
            .iter()
            .map(|s| {
                let source = self
                    .ownership
                    .stories
                    .get(s.index)
                    .filter(|r| r.index == s.index)
                    .and_then(|r| r.content)
                    .ok_or(Error::Invalid("render header slot identity"))?;
                Ok([source.start_cp, source.end_cp])
            })
            .collect()
    }

    pub(super) fn validate(&self, body_end: usize, total: usize, fonts: usize) -> Result<()> {
        self.validate_with_images(body_end, total, fonts, &Default::default())
    }

    pub(super) fn validate_with_images(
        &self,
        body_end: usize,
        total: usize,
        fonts: usize,
        image_anchors: &std::collections::BTreeSet<usize>,
    ) -> Result<()> {
        self.validate_after_notes(body_end, body_end, total, fonts, image_anchors)
    }

    pub(super) fn validate_after_notes(
        &self,
        body_end: usize,
        header_start: usize,
        total: usize,
        fonts: usize,
        image_anchors: &std::collections::BTreeSet<usize>,
    ) -> Result<()> {
        self.validate_after_notes_with_pictures(
            body_end,
            header_start,
            total,
            fonts,
            image_anchors,
            &Default::default(),
        )
    }

    pub(super) fn validate_after_notes_with_pictures(
        &self,
        body_end: usize,
        header_start: usize,
        total: usize,
        fonts: usize,
        image_anchors: &std::collections::BTreeSet<usize>,
        picture_cps: &std::collections::BTreeSet<usize>,
    ) -> Result<()> {
        let o = &self.ownership;
        if o.sections.is_empty()
            || o.sections[0].source.start_cp != 0
            || o.sections
                .last()
                .is_none_or(|s| s.source.end_cp != body_end)
            || o.sections
                .windows(2)
                .any(|w| w[0].source.end_cp != w[1].source.start_cp)
            || o.sections[0].different_first_page.is_none()
            || o.different_even_and_odd_pages.is_none()
            || o.dop_first_byte.map(|byte| byte & 1 != 0) != o.different_even_and_odd_pages
        {
            return Err(Error::Invalid("render header section identity"));
        }
        if o.source.start_cp != header_start
            || o.source.end_cp != total
            || self.source_text.encode_utf16().count() != total - header_start
        {
            return Err(Error::Invalid("render header source partition"));
        }
        let units: Vec<_> = self.source_text.encode_utf16().collect();
        let mut cp = header_start;
        for (index, story) in o.stories.iter().enumerate() {
            if story.index != index
                || story.source.start_cp != cp
                || story.source.end_cp < cp
                || story.source.end_cp > total
            {
                return Err(Error::Invalid("render header slot partition"));
            }
            if story.source.end_cp > cp {
                let end = story.source.end_cp;
                if story.guard.map(|r| [r.start_cp, r.end_cp]) != Some([end - 1, end])
                    || story.content.map(|r| [r.start_cp, r.end_cp]) != Some([cp, end - 1])
                    || units[end - 1 - header_start] != 13
                {
                    return Err(Error::Invalid("render header guard ownership"));
                }
            } else if story.guard.is_some() || story.content.is_some() {
                return Err(Error::Invalid("render empty header slot ownership"));
            }
            cp = story.source.end_cp;
        }
        if o.terminal_sentinel.map(|r| [r.start_cp, r.end_cp]) != Some([cp, total])
            || total - cp != 1
        {
            return Err(Error::Invalid("render header terminal ownership"));
        }
        let roles = [
            "even_header",
            "odd_header",
            "even_footer",
            "odd_footer",
            "first_header",
            "first_footer",
        ];
        if o.stories.len() != 6 + 6 * o.sections.len() {
            return Err(Error::Invalid("render header reference count"));
        }
        let mut inherited = [None; 6];
        for (section_index, section) in o.sections.iter().enumerate() {
            if section.index != section_index
                || section.references.len() != 6
                || section.different_first_page.is_none()
            {
                return Err(Error::Invalid("render header section identity"));
            }
            for (i, reference) in section.references.iter().enumerate() {
                let slot_index = 6 + section_index * 6 + i;
                let slot = &o.stories[slot_index];
                let local = slot.source.start_cp < slot.source.end_cp;
                if local {
                    inherited[i] = Some((slot_index, section_index));
                }
                let origin = inherited[i];
                if reference.role != roles[i]
                    || reference.local_story_index != Some(slot_index)
                    || reference.origin_story_index != origin.map(|v| v.0)
                    || reference.origin_section_index != origin.map(|v| v.1)
                    || reference.status
                        != if local {
                            "local"
                        } else if origin.is_some() {
                            "inherited"
                        } else {
                            "absent"
                        }
                {
                    return Err(Error::Invalid("render header reference ownership"));
                }
            }
        }
        if self
            .inactive_unformatted_slots
            .windows(2)
            .any(|w| w[0].index >= w[1].index)
        {
            return Err(Error::Invalid("inactive header ordering"));
        }
        for slot in &self.inactive_unformatted_slots {
            let content = o
                .stories
                .get(slot.index)
                .and_then(|s| s.content)
                .ok_or(Error::Invalid("inactive header ownership"))?;
            if !self.inactive(slot.index)
                || slot.source_cp != [content.start_cp, content.end_cp]
                || slot.uncached_fields.is_empty()
            {
                return Err(Error::Invalid("inactive header activation"));
            }
            let raw = &units[content.start_cp - header_start..content.end_cp - header_start];
            if slot.source_text.encode_utf16().collect::<Vec<_>>() != raw {
                return Err(Error::Invalid("inactive header text"));
            }
            for [a, b] in &slot.uncached_fields {
                if *a < content.start_cp
                    || *b > content.end_cp
                    || a >= b
                    || units[*a - header_start] != 19
                    || units[*b - 1 - header_start] != 21
                {
                    return Err(Error::Invalid("inactive header field envelope"));
                }
            }
        }
        let expected: Vec<_> = o
            .stories
            .iter()
            .filter(|s| {
                s.index >= 6
                    && s.content.is_some()
                    && !self
                        .inactive_unformatted_slots
                        .iter()
                        .any(|i| i.index == s.index)
            })
            .map(|s| s.index)
            .collect();
        if self.slots.iter().map(|s| s.index).collect::<Vec<_>>() != expected {
            return Err(Error::Invalid("render header content slot coverage"));
        }
        for (slot, range) in self.slots.iter().zip(self.ranges()?) {
            if !picture_cps.is_empty() {
                validate_partition_controls_with_pictures(
                    &slot.paragraphs,
                    range,
                    fonts,
                    &Default::default(),
                    image_anchors,
                    &Default::default(),
                    picture_cps,
                )?;
            } else if image_anchors.is_empty() {
                validate_partition(&slot.paragraphs, range, fonts)?;
            } else {
                validate_partition_controls(
                    &slot.paragraphs,
                    range,
                    fonts,
                    &Default::default(),
                    image_anchors,
                    &Default::default(),
                )?;
            }
            if range[0] < header_start || range[1] > total || range[0] > range[1] {
                return Err(Error::Invalid("render header content range"));
            }
            let actual: Vec<_> = slot
                .paragraphs
                .iter()
                .flat_map(|p| p.runs.iter())
                .flat_map(|r| r.text.encode_utf16())
                .collect();
            if actual != units[range[0] - header_start..range[1] - header_start] {
                return Err(Error::Invalid("render header content source text"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
