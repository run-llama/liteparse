# pptx-read

Native `.pptx` decoder. It lays out and renders every slide itself and writes the worker's
`extract` file protocol (the same `index.json` + `pages.jsonl` + raster files that
`liteparse-read` produces for PDFs), plus a screenshot-only PDF. It replaces the
LibreOffice → PDF → pdfium path for `.pptx`. Design and contract:
[`llamaparse/pptx-native/SPEC.md`](../../../llamaparse/pptx-native/SPEC.md).

## Usage

```
pptx-read -c config.json input.pptx index.json pages.jsonl
```

`config.json` is a superset of the worker's `PdfiumConfig`
(`llamaparse/worker/src/utils/pdfiumConfig.ts`); the pptx-only blocks are `presentation`,
`render`, `pdf` and `limits` (`src/config.rs`). Exit codes match `liteparse-read`: 0 when
the index was written (document errors such as `PDF_IS_PROTECTED` or
`OFFICE_FILE_IS_BROKEN` are reported inside it), 2 for a bad command line, 1 otherwise.
A panic while building one slide becomes a page error plus a `SLIDE_RENDER_FAILED`
warning; the rest of the document is still written.

Slides are built on `workers` threads (config), else `PPTX_READ_WORKERS`, else 4, capped
by the cores the process may use and by the page count; `1` runs the sequential loop.
Each thread has its own parsed trees and font-system copy; the zip, the decoded-picture
store and the layer cache are shared, and records are written in page order, so the
output is byte-identical for any count (`src/extract/parallel.rs`). Memory stays near the
sequential peak on heavy decks: pages are claimed only while their estimated trees and
first-decoded pictures fit in 192 MB, and pictures over 4 MP (or files over 16 MB) are
decoded one at a time.

## How it works

One slide → one `Scene` (display list, `src/scene.rs`) → both the raster and every JSON
output, so reported text, images and paths always match the pixels.

| Stage                                                                  | Files                                              |
| ---------------------------------------------------------------------- | -------------------------------------------------- |
| Package, XML, inheritance chain (slide → layout → master → theme)      | `package.rs`, `xml.rs`, `document.rs`, `theme.rs`  |
| Shapes, placeholders, groups, pictures, backgrounds, SmartArt drawings | `slide.rs`                                         |
| Preset/custom geometry (ECMA preset table vendored in `assets/`)       | `geom.rs`                                          |
| Fills, lines, shadows, glow and soft edges, picture recolor            | `style.rs`, `effects.rs`, `color.rs`, `recolor.rs` |
| Text: property inheritance, fonts, shaping, line breaking              | `textprops.rs`, `fonts.rs`, `textlayout.rs`        |
| Symbol fonts (Symbol, Wingdings 1–3, Webdings) remapped to Unicode     | `symbols.rs`                                       |
| Embedded fonts: EOT container, MicroType Express decompression         | `embedded_fonts.rs`, `eot.rs`, `eot/`              |
| Tables, charts (radar, surface and chartEx layouts in `chart_render/`) | `table.rs`, `chart.rs`, `chart_render.rs`          |
| EMF/WMF pictures (with extractable text)                               | `metafile.rs`, `metafile/`                         |
| SmartArt saved without its drawing, laid out from its definition       | `smartart.rs`, `smartart/`                         |
| Rasterizer (tiny-skia), page records, screenshot PDF                   | `render.rs`, `output.rs`, `pdf.rs`, `extract.rs`   |

PowerPoint, not LibreOffice, is the fidelity reference. Rules that came from measuring
against PowerPoint renders:

- Autofit (`normAutofit`) applies the stored `fontScale` and `lnSpcReduction` as saved,
  without recomputing them, and rounds each scaled font size to a whole point (18 pt at
  92.5 % draws at 17 pt). Line breaks in PowerPoint thumbnails match that, not the exact
  product or a half-point grid.
- Single line height is 1.2 × the font size; only an installed East Asian font that the
  deck itself requests keeps its taller metrics (substitutes never do, and Meiryo decks
  match PowerPoint at 1.2). Extra or reduced spacing goes above the line.
- Missing fonts use metric-compatible substitutes (Calibri → Carlito, Arial → Liberation
  Sans, ...). Where no clone exists, the substitute was picked by measured widths and by
  scoring renders against PowerPoint thumbnails (`SUBSTITUTIONS` in `src/fonts.rs`); a
  `Family*0.7` entry draws the substitute's glyphs at 0.7 × the size (Thai Office fonts).
  With no substitute, the run's PANOSE weight/serif style and a weight word in the family
  name ("SemiBold") pick the fallback.
  A text box saved with "resize shape to fit text" stores its text height in the deck's
  own fonts; when generic fallbacks overflow it, the missing fonts are retried with a
  narrower face (Barlow Semi Condensed, then Liberation Sans Narrow).
  Requested names are normalized first: the first entry of a comma list, PostScript
  names (`TimesNewRomanPS-BoldMT` → Times New Roman, bold), UI labels (`Calibri (Body)`),
  and weight words, also before a width word (`Bahnschrift SemiBold Condensed`). Family
  names match installed fonts regardless of case. A condensed name whose base family is
  installed with a `wdth` axis (Open Sans, Roboto) draws a narrowed instance; narrow
  Office faces have narrow entries in the table. With no entry, an unknown condensed
  font stays on Carlito: generic narrow faces scored worse against thumbnails.
- A line with no text takes its height from its empty runs, else from the mark that
  ends it: its `a:br`, or on the paragraph's last line the `endParaRPr` (without one the
  paragraph mark takes the last run's formatting). A shape style's `fontRef` color beats
  inherited text styles, and a table style's text color beats the master's `otherStyle`.
- Super- and subscripts draw at 2/3 of the size and do not grow the line: its height
  and percentage spacing follow the smaller size, and the raised text may stick out.
- Placeholders do not inherit `p:defaultTextStyle` (built-in 18 pt default); text boxes
  do. Text boxes on a master also take its `otherStyle`, those on layouts do not.
- A bold run in Arial Unicode MS (no bold face) is drawn regular, as PowerPoint does.
  Bold Calibri Light stays bold: PowerPoint installs disagree on it.
- Picture `a:lum` is a linear map around mid-gray. Positive contrast `c` scales by
  1/(1−c) and applies brightness before it; negative contrast scales by 1+c and damps
  brightness to bright × (1 + 0.55c), so "Washout" (+70 %/−70 %) stays a faint picture.
- Thai, Lao, Khmer and Burmese are broken at dictionary word boundaries (ICU segmenter).
- Characters a run's font lacks fall back to a face for their script first (Noto Naskh
  Arabic for Arabic, as compact as the Office fonts' own Arabic), then to a general list.
- Pattern fills (`a:pattFill`) draw their 8×8 preset tile at screen-pixel scale (6 pt),
  anchored to the slide, as PowerPoint does (`src/patterns.rs`).
- Path gradients (`a:path`) are focal: each contour's centre moves from the `fillToRect`
  focus to the tile centre while it grows to the tile's half diagonal (circle) or half
  sizes (rect, shape), so off-box foci give PowerPoint's skewed glow. A crossed inset pair
  collapses onto its first inset; exactly two stops at 0 %/100 % are spread by `s²`
  (`src/pathgrad.rs`).
- `a:grpFill` paints with the nearest enclosing group's `grpSpPr` fill (a `grpFill` group
  passes its parent's on; a group without a fill gives none), placed over the group's box,
  so a gradient spans the whole group rather than each shape.
- A picture fill turns and flips with its shape unless `rotWithShape="0"`, which keeps
  the picture upright over the turned shape's bounding box.
- chartEx frames (waterfall, funnel, box & whisker, treemap, sunburst, histogram, Pareto)
  are drawn natively, not from the `mc:Fallback` picture: that picture has no text and is
  sometimes blank (box & whisker). Missing formatting comes from the chart style part.
- Built-in table styles missing from `tableStyles.xml` come from
  `assets/presetTableStyles.xml`: all 74 styles PowerPoint ships, keyed by GUID.
- WordArt warps (`a:prstTxWarp`, `src/warp.rs`) bend glyph outlines along the preset's
  guide paths (`assets/presetTextWarpDefinitions.xml`). Two-path presets stretch each line
  to a band of the box and map it between the top and bottom guides; one-path arches keep
  the font size and stand the text on the arc (`textCircle` hangs it inside). `textPlain`
  stretches only legacy WordArt (`fromWordArt="1"`). Warped text items report the bounding
  box of the warped run.
- A slide placeholder matches its layout placeholder by `idx`, then by type, then by type
  family (`ctrTitle` ↔ `title`). One saved with `idx="4294967295"` (no parent) skips the
  layout, even one with a placeholder of its type, and inherits from the master only.
  Pictures in placeholders inherit geometry, fill, outline and effects like other shapes.
- Soft edges (`a:softEdge`) fade a shape's fill and outline, not its text, from
  transparent 0.35 × `rad` inside the outline to opaque at 1.35 × `rad` (smoothstep);
  a picture fades from its geometry. Glow (`a:glow`, shapes, pictures and text runs)
  is full color on the outline and eases out to 0.9 × `rad`; a picture's glow follows its
  transparent pixels. Both use the box-filtered silhouette as a distance
  (`src/effects.rs`, `src/render/softedge.rs`).
- Hidden shapes are not drawn, but their text is emitted as invisible text items
  (`presentation.shapes[].hidden`), as the LibreOffice PDF exposed it.
- Text drawn inside EMF/WMF pictures is emitted as text items over the picture without
  being drawn twice. EMF `GRADIENTFILL` triangles are Gouraud-shaded and, like rectangles,
  filled without anti-aliasing, as GDI does (sub-pixel bands leave no seams).
- Small caps (`cap="small"`) draw lowercase letters as capitals at 0.8 × the size.
- An underline takes `a:uFill`/`a:uLn` when set, else the text fill, and is stroked with
  the run's glyph outline when it has one.
- An italic run whose resolved face has no italic is slanted synthetically.
- With `outOfBoundsContent: "clip"`, text items wholly outside the slide are dropped (the
  LibreOffice PDF had none). A deck whose slides are all hidden emits them all, with an
  `ALL_SLIDES_HIDDEN` warning.
- Pictures `image` rejects: JPEGs above `limits.maxImagePixels` are read at 1/8 scale from
  their DC coefficients (`render/jpeg_dc.rs`); palette and CMYK+alpha TIFFs go through
  `render/tiff_extra.rs`.

The shared rasterizer accepts `render::RenderAssets` for image bytes, bounded header reads and asset sizes. The presentation document forwards these reads to its package; other document decoders can supply source-owned assets directly. Image and layer caches remain scoped to one document with immutable asset keys.

## Fonts

The production image must ship the open font set: `scripts/fetch-fonts.sh <dir>` (Google
Fonts OFL faces, TeX Gyre, Selawik, Noto script faces) plus Liberation, DejaVu and Noto
CJK. Pass the directories in `render.fontDirs` with `render.noSystemFonts: true`; any
script without a listed fallback is found by scanning every installed face.
`render.fontSubstitutions` (`{"family": "Substitute"}`) overrides the built-in table. On a
dev machine the system fonts and LibreOffice's bundled fonts are also used, so
fidelity measured there is optimistic.

Emoji draw in Noto Emoji's monochrome outlines (the rasterizer cannot draw colour emoji
fonts' bitmaps or layers), filled with the run colour. Font Awesome 4.7 is shipped for
decks that address its icons by private-use code point. Text items of a variable-font
instance report the instance's name (`Raleway-SemiBold`), not the file's default.

Fonts embedded in the deck (`p:embeddedFontLst` → `ppt/fonts/*.fntdata`) are used for
that document only, as PowerPoint does: an installed family of the same name wins, and
the embedded copy comes before the substitution table. Each face is registered under the
typeface and regular/bold/italic/boldItalic slot that `presentation.xml` names, whatever
its own name table says (embedded subsets often carry odd names); a missing slot falls
back to the nearest one. The parts are Embedded OpenType (`eot.rs`: header versions 1.0,
2.1 and 2.2, optional XOR obfuscation), almost always MicroType Express compressed
(`eot/mtx.rs`: LZCOMP with its adaptive Huffman coder; `eot/ctf.rs`: Compact Table Format
back to `glyf`/`loca`/`cvt `/`hdmx`/`VDMX` and the glyphs' PUSH data and instructions),
written from the W3C MTX and EOT submissions. Embedded East Asian faces keep the 1.2 ×
size line height. A part that does not decode is skipped with a
`FONT_EMBEDDED_UNSUPPORTED` warning; `render.useEmbeddedFonts: false` turns the feature off.

## Vendored assets

- `assets/presetShapeDefinitions.xml`: the ECMA-376 preset shape geometry table, copied
  from Apache POI (Apache-2.0).
- `assets/presetTextWarpDefinitions.xml`: the ECMA-376 text warp (WordArt) preset table,
  copied from docx4j (Apache-2.0).
- `assets/presetTableStyles.xml`: the definitions of PowerPoint's 74 built-in table
  styles, keyed by style GUID.

## Tests

`cargo test -p pptx-read`. `metafile/tests.rs` has an ignored harness
(`METAFILE_DIR`, `METAFILE_OUT`) that renders a directory of metafiles to PNG.
`eot/tests.rs` has one (`eot_corpus`, `EOT_CORPUS=<dir of .pptx>`, optional `EOT_DUMP`)
that decodes every embedded font and reports success per variant.

## Known gaps

A bold or italic run whose embedded family lacks that slot is drawn with the nearest
embedded face, slanted for italic but without synthetic emboldening; 3-D effects are flattened;
reflection is not drawn, and group-level (`grpSpPr`) glow and soft edges are ignored;
chartEx region maps keep PowerPoint's fallback picture (`CHART_PARTIAL`); 3-D surface
charts are an orthographic projection without perspective or shading. SmartArt saved without its drawing is laid out by `smartart/`
(see below); the `pyra` algorithm and the user's position tweaks (`custLinFact*`,
`custAng`, `custRadScale*`) are not followed, and a diagram whose definition is missing
falls back to an indented text list (`SMARTART_NO_FALLBACK`).

## SmartArt without a drawing

PowerPoint 2007 and some converters save SmartArt as data only (`dgm:dataModel` plus the
layout, colors and quick style parts), and PowerPoint lays it out when it opens the file.
`smartart/` does the same (warning `SMARTART_LAID_OUT`), written from ECMA-376 Part 1
§21.4: `build.rs` instantiates the layout definition over the data (`forEach`, `choose`,
`presOf`), `layout.rs` evaluates constraints and the `lin`, `snake`, `cycle`, `composite`,
`hierRoot`/`hierChild` and `conn` algorithms, `fit.rs` picks font sizes, and `emit.rs`
writes a `dsp:spTree` that `slide.rs` draws like a saved drawing. Rules measured against
PowerPoint's own drawings (decks that have both parts):

- Sizes are millimetres, font sizes points, and text margins points
  (`lMarg = 0.3 × primFontSz`).
- Constraint lists are applied twice, so a reference to a value set further down works.
  A node's own constraints and bare declarations (`<constr type="h"/>`, a zero) never
  replace a value an ancestor set.
- `lin` shrinks its children along its axis only, re-deriving sizes tied to the axis size
  (`h = 0.6 w`), and the whole subtree shrinks with each child; it ignores `sp`.
- A stack whose font-derived sizes (`h = 0.52 × primFontSz`) overflow is fixed by a
  smaller font, not by squeezing. Each group of same-named text nodes gets the largest whole
  size where all its text fits; rules apply in order, so a growth rule after a font rule
  (`primFontSz → 18`, then `h × 1.5`) only applies at that size. A word never breaks.
- Child-level text is 0.78 × the node's font size; diagram text uses the theme font of its
  quick style, not the presentation's default text style.
- Fills, lines and text colors come from the colors part by style label and
  `presStyleIdx`/`presStyleCnt` (`repeat` cycles the accents, `span` interpolates).
