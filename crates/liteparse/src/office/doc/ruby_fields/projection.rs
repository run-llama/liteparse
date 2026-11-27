use super::*;
use crate::office::doc::Transcription;

/// Restore ruby bases using the same bounded pass used for other indexed EQ literals.
pub fn apply_ruby_fields(
    input: &Transcription,
    source: &Inspection,
    ruby: &mut RubyInspection,
) -> Result<Transcription> {
    crate::office::doc::equation_fields::apply_fields(input, source, Some(ruby), None)
}
