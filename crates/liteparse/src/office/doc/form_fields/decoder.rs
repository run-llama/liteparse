use crate::office::doc::{Error, Result, bytes, u16_at, u32_at};
use serde::Serialize;

mod legacy;

#[derive(Debug, Serialize)]
pub struct FormData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legacy_encoding: Option<legacy::LegacyEncoding>,
    pub kind: &'static str,
    pub raw_bits: u16,
    pub raw_current_state: u8,
    pub default_state: Option<u16>,
    pub effective_checked: Option<bool>,
    pub selected_index: Option<u8>,
    pub selected_text: Option<String>,
    pub name: String,
    pub default_text: Option<String>,
    pub text_format: String,
    pub help_text: String,
    pub status_text: String,
    pub entry_macro: String,
    pub exit_macro: String,
    pub items: Vec<String>,
}

/// Decode the Data-stream record at CPicLocation. Lengths include the 68-byte NilPICF.
/// MS-DOC §§2.9.78–79, 2.9.158, 2.9.354 and 2.2.4 define the payload and strings.
pub(super) fn decode(data: &[u8], offset: usize, budget: &mut usize) -> Result<(FormData, usize)> {
    let length = u32_at(data, offset)? as i32;
    if !(68..=1024 * 1024).contains(&length) {
        return Err(Error::Invalid("form data record length"));
    }
    let record = bytes(data, offset, length as usize)?;
    if u16_at(record, 4)? != 68 {
        return Err(Error::Invalid("form data header size"));
    }
    *budget = budget
        .checked_sub(record.len())
        .ok_or(Error::Limit("form data bytes"))?;
    let mut reader = Reader {
        data: &record[68..],
        offset: 0,
    };
    if reader.dword()? != u32::MAX {
        return legacy::decode(&record[68..]).map(|data| (data, record.len()));
    }
    let bits = reader.word()?;
    let typ = bits & 3;
    let state = ((bits >> 2) & 31) as u8;
    let text_type = (bits >> 11) & 7;
    let cch = reader.word()?;
    let hps = reader.word()?;
    if typ > 2
        || cch > 32767
        || (typ != 0 && cch != 0)
        || (typ == 1 && !(2..=3168).contains(&hps))
        || (typ != 1 && bits & 0x400 != 0)
        || text_type > 5
        || (typ != 0 && text_type != 0)
        || (bits & 0x8000 != 0) != (typ == 2)
        || (typ == 0 && state != 0)
        || (typ == 1 && !matches!(state, 0 | 1 | 25))
    {
        return Err(Error::Invalid("form data type or state"));
    }
    let name = reader.string(20, true)?;
    let default_text = if typ == 0 {
        Some(reader.string(255, true)?)
    } else {
        None
    };
    if matches!(text_type, 3 | 4) && default_text.as_ref().is_some_and(|s| !s.is_empty()) {
        return Err(Error::Invalid("form date default"));
    }
    let default_state = if typ != 0 { Some(reader.word()?) } else { None };
    if typ == 1 && default_state.is_some_and(|s| s > 1) {
        return Err(Error::Invalid("form checkbox default"));
    }
    let text_format = reader.string(64, true)?;
    if typ != 0 && !text_format.is_empty() {
        return Err(Error::Invalid("form text format"));
    }
    let help_text = reader.string(255, true)?;
    let status_text = reader.string(138, true)?;
    let entry_macro = reader.string(32, true)?;
    let exit_macro = reader.string(32, true)?;
    let mut items = Vec::new();
    if typ == 2 {
        if reader.word()? != u16::MAX {
            return Err(Error::Invalid("form dropdown Unicode table"));
        }
        let count = reader.word()? as usize;
        if count > 25 || reader.word()? != 0 {
            return Err(Error::Invalid("form dropdown table"));
        }
        for _ in 0..count {
            items.push(reader.string(65535, false)?);
        }
        if default_state.is_none_or(|s| s as usize >= count)
            || (state != 25 && state as usize >= count)
        {
            return Err(Error::Invalid("form dropdown index"));
        }
    }
    if reader.offset != reader.data.len() {
        return Err(Error::Unsupported("form data trailing bytes"));
    }
    let selected_index = (typ == 2 && state != 25).then_some(state);
    Ok((
        FormData {
            legacy_encoding: None,
            kind: ["text", "checkbox", "dropdown"][typ as usize],
            raw_bits: bits,
            raw_current_state: state,
            default_state,
            effective_checked: (typ == 1).then_some(state == 1),
            selected_index,
            selected_text: selected_index.map(|i| items[i as usize].clone()),
            name,
            default_text,
            text_format,
            help_text,
            status_text,
            entry_macro,
            exit_macro,
            items,
        },
        record.len(),
    ))
}

struct Reader<'a> {
    data: &'a [u8],
    offset: usize,
}
impl Reader<'_> {
    fn word(&mut self) -> Result<u16> {
        let v = u16_at(self.data, self.offset)?;
        self.offset += 2;
        Ok(v)
    }
    fn dword(&mut self) -> Result<u32> {
        let v = u32_at(self.data, self.offset)?;
        self.offset += 4;
        Ok(v)
    }
    fn string(&mut self, limit: usize, terminated: bool) -> Result<String> {
        let count = self.word()? as usize;
        if count > limit {
            return Err(Error::Limit("form string units"));
        }
        let raw = bytes(self.data, self.offset, count * 2)?;
        self.offset += count * 2;
        let units: Vec<_> = raw
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        let text = String::from_utf16(&units).map_err(|_| Error::Invalid("form string Unicode"))?;
        if terminated && self.word()? != 0 {
            return Err(Error::Invalid("form string terminator"));
        }
        Ok(text)
    }
}
