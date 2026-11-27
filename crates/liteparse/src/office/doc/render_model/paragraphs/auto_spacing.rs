//! Ordered auto-spacing switches are independent of stored explicit distances.
use crate::office::doc::{Error, Result, formatting::PropertyModifier};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct AutoSpacing {
    pub before: bool,
    pub after: bool,
}

pub fn resolve(properties: &[PropertyModifier]) -> Result<AutoSpacing> {
    let mut result = AutoSpacing::default();
    for property in properties {
        let target = match property.sprm {
            0x245b => &mut result.before,
            0x245c => &mut result.after,
            _ => continue,
        };
        *target = match property.operand.as_slice() {
            [0] => false,
            [1] => true,
            _ => return Err(Error::Invalid("render paragraph auto-spacing Bool8")),
        };
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
