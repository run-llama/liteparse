//! Source-resolved binary Word layout properties and their qualification evidence.
//! This module retains binary units and source positions before shared-model conversion.
//! A decoded geometry record alone does not qualify a document for page rendering.
use serde::Serialize;

pub mod paragraphs;
pub mod plain;
pub mod runs;
pub mod sections;
pub mod settings;
pub mod tables;

/// The modifier index is relative to the source object's ordered property list.
/// Missing properties and cross-property constraints have no modifier index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PropertyIssue {
    pub modifier_index: Option<usize>,
    pub sprm: Option<u16>,
    pub reason: &'static str,
}

/// A source value retains whether it came from an explicit modifier or a specified default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Value<T> {
    pub value: T,
    pub modifier_index: Option<usize>,
}
impl<T> Value<T> {
    fn defaulted(value: T) -> Self {
        Self {
            value,
            modifier_index: None,
        }
    }
    fn explicit(value: T, index: usize) -> Self {
        Self {
            value,
            modifier_index: Some(index),
        }
    }
}
