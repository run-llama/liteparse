//! Native `.pptx` decoder. Reads a presentation, lays out and renders every slide,
//! and describes each one in the extract format (an index object plus one page
//! record per slide: text runs, paths, pictures, optional screenshot), plus an
//! optional screenshot-only PDF. See `README.md`.

pub mod chart;
pub mod chart_render;
pub mod color;
pub mod config;
pub mod document;
pub mod effects;
pub mod embedded_fonts;
pub mod eot;
pub mod error;
pub mod extract;
pub mod fonts;
pub mod geom;
pub mod layers;
pub mod math;
pub mod metafile;
pub mod notes;
pub mod output;
pub mod package;
pub mod pathgrad;
pub mod patterns;
pub mod pdf;
pub mod peak_rss;
pub mod raster;
pub mod recolor;
pub mod render;
pub mod scene;
pub mod shots;
pub mod slide;
pub mod smartart;
pub mod style;
pub mod symbols;
pub mod table;
pub mod textlayout;
pub mod textprops;
pub mod theme;
pub mod warp;
pub mod xml;
