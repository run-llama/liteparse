//! Native `.docx` decoder. Parses WordprocessingML, lays the document out into
//! pages the way Microsoft Word does (styles, line breaking, pagination,
//! tables, headers and footers), and describes each page in the extract format
//! (an index object plus one page record per page), plus an optional
//! screenshot-only PDF. Pages are drawn and reported through the `.pptx`
//! decoder's scene, renderer and outputs. See `README.md`.

mod comments;
pub mod deadline;
pub mod decor;
mod doc_props;
mod embedded_fonts;
pub mod extract;
pub mod graphics;
pub mod hyph;
pub mod layout;
pub mod memory;
pub mod metrics;
pub mod model;
pub mod numbering;
pub mod paginate;
pub mod parse;
pub mod props;
pub mod shapes;
pub mod styles;
pub mod text;
mod theme_colors;
pub mod wrap;

#[cfg(test)]
mod testutil;
