//! Shared grammar for section operands used by source-property consumers.
/// Word 97–2007 specification pp.63,95,172: OLST80 has nine 16-byte ANLV80
/// records, four flag bytes and 32 UTF-16 units (212 bytes). SOlstCv supplies
/// nine four-byte COLORREF values. The SPRM length byte is retained in operands.
/// <https://www.loc.gov/preservation/digital/formats/digformatspecs/Word97-2007BinaryFileFormat(doc>)Specification.pdf
pub(crate) fn outline_operand(sprm: u16, operand: &[u8]) -> bool {
    let payload = match sprm {
        0xd202 => 9 * 16 + 4 + 32 * 2,
        0xd238 => 9 * 4,
        _ => return false,
    };
    operand.len() == payload + 1 && operand[0] as usize == payload
}

/// MS-DOC 2.2.5 recommends skipping unsupported Prls; Appendix A note 6
/// specifies that every listed Word version skips properties outside its ranges.
/// D1FF has section ispmd 0x1ff, outside every listed Word range (maximum 0x44).
/// Retain the observed, exactly framed operand while leaving other properties guarded.
pub(crate) fn ignored_d1ff(sprm: u16, operand: &[u8]) -> bool {
    sprm == 0xd1ff && operand == [2, 3, 0]
}
