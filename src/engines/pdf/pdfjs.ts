import fs from "node:fs/promises";
import {
  PdfEngine,
  PdfDocument,
  PageData,
  Image,
  Annotation,
  BoundingBox,
  ExtractOptions,
} from "./interface.js";
import { TextItem } from "../../core/types.js";
import { PdfiumRenderer } from "./pdfium-renderer.js";
import { importPdfJs } from "./pdfjsImporter.js";

/** PDF.js internal document type - opaque to our code */
interface PdfJsDocument {
  numPages: number;
  getPage(pageNum: number): Promise<PdfJsPage>;
  getMetadata(): Promise<unknown>;
  destroy(): Promise<void>;
}

/** PDF.js internal page type */
interface PdfJsPage {
  getViewport(params: { scale: number }): PdfJsViewport;
  getTextContent(): Promise<PdfJsTextContent>;
  cleanup(): Promise<void>;
}

/** PDF.js viewport type */
interface PdfJsViewport {
  width: number;
  height: number;
  transform: number[];
}

/** PDF.js text content type */
interface PdfJsTextContent {
  items: PdfJsTextItem[];
}

/** PDF.js text item type */
interface PdfJsTextItem {
  str: string;
  transform: number[];
  width: number;
  height: number;
  fontName?: string;
}

/** Extended PdfDocument with internal PDF.js document reference */
interface PdfJsExtendedDocument extends PdfDocument {
  _pdfDocument: PdfJsDocument;
}

// Dynamic import of PDF.js
const { fn: getDocument, dir: PDFJS_DIR } = await importPdfJs();

const CMAP_URL = `${PDFJS_DIR}/cmaps/`;
const STANDARD_FONT_DATA_URL = `${PDFJS_DIR}/standard_fonts/`;
const WASM_URL = `${PDFJS_DIR}/`;
const CMAP_PACKED = true;

/**
 * Extract rotation angle in degrees from PDF transformation matrix
 * Matrix format: [a, b, c, d, e, f] where rotation is atan2(b, a)
 */
function getRotation(transform: number[]): number {
  return Math.atan2(transform[1], transform[0]) * (180 / Math.PI);
}

/**
 * Multiply two transformation matrices
 */
function multiplyMatrices(m1: number[], m2: number[]): number[] {
  return [
    m1[0] * m2[0] + m1[2] * m2[1],
    m1[1] * m2[0] + m1[3] * m2[1],
    m1[0] * m2[2] + m1[2] * m2[3],
    m1[1] * m2[2] + m1[3] * m2[3],
    m1[0] * m2[4] + m1[2] * m2[5] + m1[4],
    m1[1] * m2[4] + m1[3] * m2[5] + m1[5],
  ];
}

/**
 * Apply transformation matrix to a point
 */
function applyTransformation(
  point: { x: number; y: number },
  transform: number[]
): { x: number; y: number } {
  return {
    x: point.x * transform[0] + point.y * transform[2] + transform[4],
    y: point.x * transform[1] + point.y * transform[3] + transform[5],
  };
}

// Pre-compiled regex patterns for string decoding
const BUGGY_FONT_MARKER_CHECK = ":->|>";
const PIPE_PATTERN_REGEX = /\s*\|([^|])\|\s*/g;

/**
 * Adobe Glyph List subset: maps standard PostScript glyph names to Unicode characters.
 *
 * When PDF.js detects a "buggy" font (one whose ToUnicode/encoding maps glyphs to
 * control characters or PUA code points), it emits markers containing the glyph's
 * original char code AND the glyph name from the font's /Differences or /Encoding
 * dictionary. This map resolves those glyph names to correct Unicode characters.
 *
 * This is a ~200-entry subset of the full Adobe Glyph List (~4,300 entries).
 * The full canonical source is: https://github.com/adobe-type-tools/agl-aglfn
 * (see glyphlist.txt). Our subset covers basic Latin, digits, ligatures, punctuation,
 * typographic characters, Greek, math symbols, and common accented Latin. Glyph names
 * not in this subset fall through to the uniXXXX convention and ASCII-range fallbacks
 * in resolveGlyphName(). Add entries here if a PDF's buggy font uses a standard glyph
 * name that isn't covered and doesn't match those fallbacks.
 */
const ADOBE_GLYPH_MAP: Record<string, string> = {
  // Basic Latin letters
  A: "A",
  B: "B",
  C: "C",
  D: "D",
  E: "E",
  F: "F",
  G: "G",
  H: "H",
  I: "I",
  J: "J",
  K: "K",
  L: "L",
  M: "M",
  N: "N",
  O: "O",
  P: "P",
  Q: "Q",
  R: "R",
  S: "S",
  T: "T",
  U: "U",
  V: "V",
  W: "W",
  X: "X",
  Y: "Y",
  Z: "Z",
  a: "a",
  b: "b",
  c: "c",
  d: "d",
  e: "e",
  f: "f",
  g: "g",
  h: "h",
  i: "i",
  j: "j",
  k: "k",
  l: "l",
  m: "m",
  n: "n",
  o: "o",
  p: "p",
  q: "q",
  r: "r",
  s: "s",
  t: "t",
  u: "u",
  v: "v",
  w: "w",
  x: "x",
  y: "y",
  z: "z",
  // Digits
  zero: "0",
  one: "1",
  two: "2",
  three: "3",
  four: "4",
  five: "5",
  six: "6",
  seven: "7",
  eight: "8",
  nine: "9",
  // Ligatures (Unicode presentation forms — decomposed later by stripControlChars)
  fi: "\uFB01",
  fl: "\uFB02",
  ff: "\uFB00",
  ffi: "\uFB03",
  ffl: "\uFB04",
  // Punctuation and symbols
  space: " ",
  period: ".",
  comma: ",",
  colon: ":",
  semicolon: ";",
  hyphen: "-",
  minus: "\u2212",
  slash: "/",
  question: "?",
  dollar: "$",
  parenleft: "(",
  parenright: ")",
  asterisk: "*",
  plus: "+",
  equal: "=",
  numbersign: "#",
  percent: "%",
  ampersand: "&",
  at: "@",
  exclam: "!",
  bracketleft: "[",
  bracketright: "]",
  braceleft: "{",
  braceright: "}",
  underscore: "_",
  quotedbl: '"',
  quotesingle: "'",
  backslash: "\\",
  bar: "|",
  asciitilde: "~",
  asciicircum: "^",
  grave: "`",
  less: "<",
  greater: ">",
  // Typographic
  quoteright: "\u2019",
  quoteleft: "\u2018",
  quotedblleft: "\u201C",
  quotedblright: "\u201D",
  quotesinglbase: "\u201A",
  quotedblbase: "\u201E",
  endash: "\u2013",
  emdash: "\u2014",
  bullet: "\u2022",
  ellipsis: "\u2026",
  dagger: "\u2020",
  daggerdbl: "\u2021",
  guilsinglleft: "\u2039",
  guilsinglright: "\u203A",
  guillemotleft: "\u00AB",
  guillemotright: "\u00BB",
  trademark: "\u2122",
  registered: "\u00AE",
  copyright: "\u00A9",
  // Greek
  Alpha: "\u0391",
  Beta: "\u0392",
  Gamma: "\u0393",
  Delta: "\u2206",
  Epsilon: "\u0395",
  Zeta: "\u0396",
  Eta: "\u0397",
  Theta: "\u0398",
  Iota: "\u0399",
  Kappa: "\u039A",
  Lambda: "\u039B",
  Mu: "\u039C",
  Nu: "\u039D",
  Xi: "\u039E",
  Omicron: "\u039F",
  Pi: "\u03A0",
  Rho: "\u03A1",
  Sigma: "\u03A3",
  Tau: "\u03A4",
  Upsilon: "\u03A5",
  Phi: "\u03A6",
  Chi: "\u03A7",
  Psi: "\u03A8",
  Omega: "\u2126",
  alpha: "\u03B1",
  beta: "\u03B2",
  gamma: "\u03B3",
  delta: "\u03B4",
  epsilon: "\u03B5",
  zeta: "\u03B6",
  eta: "\u03B7",
  theta: "\u03B8",
  iota: "\u03B9",
  kappa: "\u03BA",
  lambda: "\u03BB",
  mu: "\u00B5",
  nu: "\u03BD",
  xi: "\u03BE",
  omicron: "\u03BF",
  pi: "\u03C0",
  rho: "\u03C1",
  sigma: "\u03C3",
  tau: "\u03C4",
  upsilon: "\u03C5",
  phi: "\u03C6",
  chi: "\u03C7",
  psi: "\u03C8",
  omega: "\u03C9",
  // Math symbols
  greaterequal: "\u2265",
  lessequal: "\u2264",
  notequal: "\u2260",
  plusminus: "\u00B1",
  multiply: "\u00D7",
  divide: "\u00F7",
  infinity: "\u221E",
  summation: "\u2211",
  integral: "\u222B",
  partialdiff: "\u2202",
  radical: "\u221A",
  approxequal: "\u2248",
  degree: "\u00B0",
  // Accented Latin (common)
  Aacute: "\u00C1",
  Agrave: "\u00C0",
  Acircumflex: "\u00C2",
  Atilde: "\u00C3",
  Adieresis: "\u00C4",
  Aring: "\u00C5",
  Eacute: "\u00C9",
  Egrave: "\u00C8",
  Ecircumflex: "\u00CA",
  Edieresis: "\u00CB",
  Iacute: "\u00CD",
  Igrave: "\u00CC",
  Icircumflex: "\u00CE",
  Idieresis: "\u00CF",
  Oacute: "\u00D3",
  Ograve: "\u00D2",
  Ocircumflex: "\u00D4",
  Otilde: "\u00D5",
  Odieresis: "\u00D6",
  Uacute: "\u00DA",
  Ugrave: "\u00D9",
  Ucircumflex: "\u00DB",
  Udieresis: "\u00DC",
  Ntilde: "\u00D1",
  Ccedilla: "\u00C7",
  Scaron: "\u0160",
  Zcaron: "\u017D",
  aacute: "\u00E1",
  agrave: "\u00E0",
  acircumflex: "\u00E2",
  atilde: "\u00E3",
  adieresis: "\u00E4",
  aring: "\u00E5",
  eacute: "\u00E9",
  egrave: "\u00E8",
  ecircumflex: "\u00EA",
  edieresis: "\u00EB",
  iacute: "\u00ED",
  igrave: "\u00EC",
  icircumflex: "\u00EE",
  idieresis: "\u00EF",
  oacute: "\u00F3",
  ograve: "\u00F2",
  ocircumflex: "\u00F4",
  otilde: "\u00F5",
  odieresis: "\u00F6",
  uacute: "\u00FA",
  ugrave: "\u00F9",
  ucircumflex: "\u00FB",
  udieresis: "\u00FC",
  ntilde: "\u00F1",
  ccedilla: "\u00E7",
  scaron: "\u0161",
  zcaron: "\u017E",
  ydieresis: "\u00FF",
  // Miscellaneous
  AE: "\u00C6",
  ae: "\u00E6",
  OE: "\u0152",
  oe: "\u0153",
  Eth: "\u00D0",
  eth: "\u00F0",
  Thorn: "\u00DE",
  thorn: "\u00FE",
  germandbls: "\u00DF",
  dotlessi: "\u0131",
  section: "\u00A7",
  paragraph: "\u00B6",
  currency: "\u00A4",
  cent: "\u00A2",
  sterling: "\u00A3",
  yen: "\u00A5",
  Euro: "\u20AC",
  logicalnot: "\u00AC",
  nbspace: "\u00A0",
};

/**
 * Resolve a glyph name to its Unicode character using the Adobe Glyph List.
 * Handles standard names, the "uniXXXX" convention, and underscore-separated
 * composite names (e.g., "f_i" → resolve "f" + "i" = "fi").
 */
function resolveGlyphName(glyphName: string): string | null {
  if (glyphName in ADOBE_GLYPH_MAP) return ADOBE_GLYPH_MAP[glyphName];

  // Handle "uniXXXX" convention (e.g., "uni00A0" → U+00A0)
  if (glyphName.startsWith("uni") && glyphName.length === 7) {
    const code = parseInt(glyphName.slice(3), 16);
    if (!isNaN(code) && code > 0) return String.fromCharCode(code);
  }

  // Handle underscore-separated composite names (e.g., "f_i" → "fi", "f_f_i" → "ffi")
  // Some fonts use this convention instead of standard ligature names
  if (glyphName.includes("_")) {
    const parts = glyphName.split("_");
    const resolved = parts.map((p) => resolveGlyphName(p));
    if (resolved.every((r) => r !== null)) {
      return resolved.join("");
    }
  }

  return null;
}

/**
 * Decode buggy font markers emitted by patched PDF.js.
 *
 * Marker format: :->|>_<glyphId>_<fontCharCode>@<glyphName>@<|<-:
 * The glyph name is delimited by @ instead of _ because some fonts use
 * non-standard glyph names containing underscores (e.g., "f_i" for "fi").
 *
 * Resolution strategy:
 * 1. Use glyph name from font's /Differences or /Encoding dictionary
 * 2. Fall back to glyphId if it's in printable ASCII range (32-126)
 * 3. Drop the character if neither works (better than guessing)
 */
const BUGGY_FONT_MARKER_RE = /:->\|>_(\d+)_\d+@([^@]*)@<\|<-:/g;

function decodeBuggyFontMarkers(str: string): string {
  return str.replace(BUGGY_FONT_MARKER_RE, (_match, glyphIdStr: string, glyphName: string) => {
    // Priority 1: Resolve via glyph name from font metadata
    if (glyphName) {
      const resolved = resolveGlyphName(glyphName);
      if (resolved) return resolved;
    }

    // Priority 2: If glyphId is in printable ASCII range, use it directly
    const glyphId = parseInt(glyphIdStr);
    if (glyphId >= 32 && glyphId <= 126) {
      return String.fromCharCode(glyphId);
    }

    // Priority 3: Drop unresolvable characters
    return "";
  });
}

/**
 * Windows-1252 to Unicode mapping for the C1 control range (0x80-0x9F).
 *
 * Many PDFs encode smart quotes, em-dashes, and other typographic characters
 * using Windows-1252 byte values. When PDF.js decodes these without a proper
 * ToUnicode map, the raw byte values end up in the 0x80-0x9F range — which is
 * technically the C1 control character block in Unicode. Rather than stripping
 * them (which loses apostrophes, quotes, dashes, etc.), we map them to their
 * correct Unicode equivalents.
 */
const WINDOWS_1252_TO_UNICODE: Record<number, string> = {
  0x80: "\u20AC", // €
  0x82: "\u201A", // ‚
  0x83: "\u0192", // ƒ
  0x84: "\u201E", // „
  0x85: "\u2026", // …
  0x86: "\u2020", // †
  0x87: "\u2021", // ‡
  0x88: "\u02C6", // ˆ
  0x89: "\u2030", // ‰
  0x8a: "\u0160", // Š
  0x8b: "\u2039", // ‹
  0x8c: "\u0152", // Œ
  0x8e: "\u017D", // Ž
  0x91: "\u2018", // '
  0x92: "\u2019", // ' (right single quote / apostrophe)
  0x93: "\u201C", // "
  0x94: "\u201D", // "
  0x95: "\u2022", // •
  0x96: "\u2013", // –
  0x97: "\u2014", // —
  0x98: "\u02DC", // ˜
  0x99: "\u2122", // ™
  0x9a: "\u0161", // š
  0x9b: "\u203A", // ›
  0x9c: "\u0153", // œ
  0x9e: "\u017E", // ž
  0x9f: "\u0178", // Ÿ
};

/**
 * Unicode ligature decomposition map.
 * PDF fonts often use ligature glyphs; decomposing them to plain ASCII
 * ensures the text is searchable and NLP-friendly.
 */
const LIGATURE_MAP: Record<string, string> = {
  "\uFB00": "ff",
  "\uFB01": "fi",
  "\uFB02": "fl",
  "\uFB03": "ffi",
  "\uFB04": "ffl",
  "\uFB05": "st",
  "\uFB06": "st",
};

/**
 * Strip C0 control characters from text (except common whitespace),
 * map C1 control range (0x80-0x9F) to proper Unicode via Windows-1252,
 * and decompose Unicode ligatures to plain text.
 */
function stripControlChars(str: string): string {
  let result = "";
  for (const char of str) {
    const code = char.charCodeAt(0);

    // Decompose Unicode ligatures (fi, fl, ff, ffi, ffl, st)
    if (LIGATURE_MAP[char]) {
      result += LIGATURE_MAP[char];
      continue;
    }

    // Map Windows-1252 C1 range to proper Unicode (smart quotes, em-dashes, etc.)
    if (code >= 0x80 && code <= 0x9f) {
      const mapped = WINDOWS_1252_TO_UNICODE[code];
      if (mapped) {
        result += mapped;
      }
      // Undefined C1 positions (0x81, 0x8D, 0x8F, 0x90) are dropped
      continue;
    }

    // Skip C0 controls (except tab, newline, carriage return)
    if (code >= 0x00 && code <= 0x1f && code !== 0x09 && code !== 0x0a && code !== 0x0d) {
      continue;
    }

    result += char;
  }
  return result;
}

/**
 * Detect garbled text from fonts with corrupted ToUnicode mappings.
 *
 * When PDF fonts lack proper ToUnicode maps, PDF.js may output characters
 * mapped to unexpected Unicode code points. Common patterns include:
 *
 * 1. Private Use Area (PUA) characters - fonts often map glyphs here
 * 2. Mix of unrelated scripts (Arabic + Latin Extended in English text)
 * 3. Rare/obscure Unicode blocks appearing in normal text
 * 4. Control characters (when text is predominantly control chars)
 *
 * Returns true if the string appears to be garbled font output.
 */
function isGarbledFontOutput(str: string): boolean {
  if (str.length < 3) return false;

  let privateUseCount = 0;
  let arabicCount = 0;
  let latinExtendedCount = 0;
  let basicLatinLetterCount = 0;
  let suspiciousCount = 0; // Other suspicious Unicode ranges
  let controlCharCount = 0; // C0/C1 control characters
  let normalCharCount = 0; // Normal printable characters

  for (const char of str) {
    const code = char.charCodeAt(0);

    // C0 control characters (0x00-0x1F) except common whitespace (tab, newline, carriage return)
    if (code >= 0x00 && code <= 0x1f && code !== 0x09 && code !== 0x0a && code !== 0x0d) {
      controlCharCount++;
    }
    // C1 range (0x80-0x9F): only count as control chars if NOT a valid Windows-1252 character.
    // Many PDFs use Windows-1252 encoding for smart quotes, em-dashes, etc.
    else if (code >= 0x80 && code <= 0x9f) {
      if (WINDOWS_1252_TO_UNICODE[code]) {
        normalCharCount++; // Valid Windows-1252 char (smart quote, dash, etc.)
      } else {
        controlCharCount++; // Undefined C1 position — likely garbled
      }
    }
    // Private Use Area (U+E000-U+F8FF) - almost always garbled
    else if (code >= 0xe000 && code <= 0xf8ff) {
      privateUseCount++;
    }
    // Arabic block (0x600-0x6FF) and Arabic Extended (0x750-0x77F, 0x8A0-0x8FF)
    else if (
      (code >= 0x600 && code <= 0x6ff) ||
      (code >= 0x750 && code <= 0x77f) ||
      (code >= 0x8a0 && code <= 0x8ff)
    ) {
      arabicCount++;
    }
    // Latin Extended-A (0x100-0x17F), Latin Extended-B (0x180-0x24F),
    // Latin Extended Additional (0x1E00-0x1EFF)
    else if ((code >= 0x100 && code <= 0x24f) || (code >= 0x1e00 && code <= 0x1eff)) {
      latinExtendedCount++;
    }
    // Basic Latin letters (a-z, A-Z)
    else if ((code >= 0x41 && code <= 0x5a) || (code >= 0x61 && code <= 0x7a)) {
      basicLatinLetterCount++;
      normalCharCount++;
    }
    // Suspicious ranges that rarely appear in normal text:
    // - Syriac (0x700-0x74F)
    // - Thaana (0x780-0x7BF)
    // - NKo (0x7C0-0x7FF)
    // - Samaritan (0x800-0x83F)
    // - Specials (0xFFF0-0xFFFF)
    // - Geometric Shapes (0x25A0-0x25FF) in running text
    // - Box Drawing (0x2500-0x257F) in running text
    // - Combining Diacritical Marks alone (0x0300-0x036F)
    else if (
      (code >= 0x700 && code <= 0x7ff) || // Syriac, Thaana, NKo
      (code >= 0x800 && code <= 0x83f) || // Samaritan
      (code >= 0xfff0 && code <= 0xffff) || // Specials
      (code >= 0x2500 && code <= 0x25ff) || // Box drawing, geometric shapes
      (code >= 0x0300 && code <= 0x036f) // Combining marks (suspicious if frequent)
    ) {
      suspiciousCount++;
    }
    // Normal printable characters (digits, punctuation, common symbols, space)
    else if ((code >= 0x20 && code <= 0x7e) || code === 0x09 || code === 0x0a || code === 0x0d) {
      normalCharCount++;
    }
  }

  const totalChars = str.length;

  // Text is predominantly control characters - definitely garbled
  // This catches cases like more_hard_2.pdf where text is entirely control chars
  if (controlCharCount > 0 && controlCharCount > normalCharCount) {
    return true;
  }

  // Private Use Area characters are almost always garbled fonts
  if (privateUseCount >= 2) {
    return true;
  }

  // Mix of Arabic AND Latin Extended is extremely rare in legitimate text
  if (arabicCount >= 2 && latinExtendedCount >= 2) {
    return true;
  }

  // High concentration of suspicious characters
  if (suspiciousCount >= 3 || suspiciousCount > totalChars * 0.2) {
    return true;
  }

  // Text predominantly Latin Extended with very few basic Latin letters
  // (legitimate Latin-script text would have mostly basic Latin)
  if (latinExtendedCount > totalChars * 0.3 && basicLatinLetterCount < totalChars * 0.2) {
    return true;
  }

  // Mix of Arabic/suspicious with Latin Extended (script mixing)
  if ((arabicCount >= 1 || suspiciousCount >= 1) && latinExtendedCount >= 3) {
    return true;
  }

  return false;
}

export class PdfJsEngine implements PdfEngine {
  name = "pdfjs";
  private pdfiumRenderer: PdfiumRenderer | null = null;
  private currentPdfPath: string | null = null;
  private currentPdfData: Uint8Array | null = null;

  async loadDocument(input: string | Uint8Array, password?: string): Promise<PdfDocument> {
    let data: Uint8Array;
    if (typeof input === "string") {
      data = new Uint8Array(await fs.readFile(input));
      this.currentPdfPath = input;
    } else {
      // pdf.js requires a plain Uint8Array, not a Buffer subclass. Copy
      // rather than view-on-input, because pdf.js transfers the underlying
      // ArrayBuffer to its worker — which would detach the caller's view
      // and any downstream consumer (e.g. the OCR renderer) would see zero
      // bytes. A fresh Uint8Array owns its own ArrayBuffer.
      data = new Uint8Array(input);
      this.currentPdfPath = null;
    }

    // Store data for buffer-based rendering. Keep a separate copy so the
    // one we hand to pdf.js can be transferred without affecting this one.
    this.currentPdfData = new Uint8Array(data);

    const loadingTask = getDocument({
      data,
      password,
      cMapUrl: CMAP_URL,
      cMapPacked: CMAP_PACKED,
      standardFontDataUrl: STANDARD_FONT_DATA_URL,
      wasmUrl: WASM_URL,
      verbosity: 0, // VerbosityLevel.ERRORS — suppress Type3 font warnings
    });

    let pdfDocument: PdfJsDocument;
    try {
      pdfDocument = await loadingTask.promise;
    } catch (error: unknown) {
      const message = error instanceof Error ? error.message : String(error);
      if (message.includes("password") || message.includes("Password")) {
        if (password) {
          throw new Error(
            "Incorrect password for this PDF. Please check the password and try again.",
            { cause: error }
          );
        } else {
          throw new Error(
            "This PDF is password-protected. Use --password <password> to provide the document password.",
            { cause: error }
          );
        }
      }
      throw error;
    }

    const metadata = await pdfDocument.getMetadata();

    return {
      numPages: pdfDocument.numPages,
      data,
      metadata,
      _pdfDocument: pdfDocument,
    } as PdfJsExtendedDocument;
  }

  async extractPage(
    doc: PdfDocument,
    pageNum: number,
    options?: ExtractOptions
  ): Promise<PageData> {
    const pdfDocument = (doc as PdfJsExtendedDocument)._pdfDocument;
    const page = await pdfDocument.getPage(pageNum);

    // Get viewport
    const viewport = page.getViewport({ scale: 1.0 });

    // Extract text content
    const textContent = await page.getTextContent();
    const viewportWidth = viewport.width;
    const viewportHeight = viewport.height;
    const viewportTransform = viewport.transform;

    const textItems: TextItem[] = [];
    const garbledTextRegions: BoundingBox[] = [];

    for (const item of textContent.items) {
      // Skip items with zero dimensions
      if (item.height === 0 || item.width === 0) continue;

      // Apply viewport transformation to convert PDF coordinates to screen coordinates
      // This properly handles Y-axis flip (PDF is bottom-up, screen is top-down)
      const cm = multiplyMatrices(viewportTransform, item.transform);

      // Get lower-left corner (text space origin)
      const ll = applyTransformation({ x: 0, y: 0 }, cm);
      const scaleX = Math.sqrt(item.transform[0] ** 2 + item.transform[1] ** 2);
      const scaleY = Math.sqrt(item.transform[2] ** 2 + item.transform[3] ** 2);
      const ur = applyTransformation({ x: item.width / scaleX, y: item.height / scaleY }, cm);

      const left = Math.min(ll.x, ur.x);
      const right = Math.max(ll.x, ur.x);
      const top = Math.min(ll.y, ur.y);
      const bottom = Math.max(ll.y, ur.y);

      // Skip items that are off-page (negative coordinates or beyond page bounds)
      if (top < 0 || left < 0 || top > viewportHeight || left > viewportWidth) continue;

      const width = right - left;
      const height = bottom - top;

      // Get rotation angle from the transformation matrix
      let rotation = getRotation(cm);
      if (rotation < 0) rotation += 360;

      // Decode buggy font markers using glyph names from font metadata
      let decodedStr = item.str;
      if (decodedStr.includes(BUGGY_FONT_MARKER_CHECK)) {
        BUGGY_FONT_MARKER_RE.lastIndex = 0;
        decodedStr = decodeBuggyFontMarkers(decodedStr);
      }

      // Handle pipe-separated characters: " |a|  |r|  |X| " -> "arX"
      if (decodedStr.includes("|")) {
        PIPE_PATTERN_REGEX.lastIndex = 0;
        const matches = [...decodedStr.matchAll(PIPE_PATTERN_REGEX)];
        if (matches.length > 0) {
          decodedStr = matches.map((m) => m[1]).join("");
        }
      }

      // Skip garbled text from fonts with corrupted ToUnicode mappings
      if (isGarbledFontOutput(decodedStr)) {
        garbledTextRegions.push({ x: left, y: top, width, height });
        continue;
      }

      // Strip remaining control characters, map Windows-1252, decompose ligatures
      decodedStr = stripControlChars(decodedStr);

      textItems.push({
        str: decodedStr,
        x: left,
        y: top,
        width,
        height,
        w: width,
        h: height,
        r: rotation,
        fontName: item.fontName,
        fontSize: Math.sqrt(
          item.transform[0] * item.transform[0] + item.transform[1] * item.transform[1]
        ),
        confidence: 1.0,
      });
    }

    let images: Image[] = [];
    if (options?.extractImages !== false) {
      try {
        const pdfInput = this.currentPdfPath || this.currentPdfData || doc.data;
        if (!this.pdfiumRenderer) {
          this.pdfiumRenderer = new PdfiumRenderer();
          await this.pdfiumRenderer.loadDocument(pdfInput);
        }
        const imageBounds = await this.pdfiumRenderer.extractImageBounds(pdfInput, pageNum);
        images = imageBounds.map((bounds) => ({
          x: bounds.x,
          y: bounds.y,
          width: bounds.width,
          height: bounds.height,
        }));
      } catch {
        // Image extraction is best-effort
      }
    }

    // Skip annotation extraction - not currently used in processing pipeline
    // Can be re-enabled if needed for link extraction, etc.
    const annotations: Annotation[] = [];

    await page.cleanup();

    return {
      pageNum,
      width: viewport.width,
      height: viewport.height,
      textItems,
      images,
      annotations,
      garbledTextRegions: garbledTextRegions.length > 0 ? garbledTextRegions : undefined,
    };
  }

  async extractAllPages(
    doc: PdfDocument,
    maxPages?: number,
    targetPages?: string,
    options?: ExtractOptions
  ): Promise<PageData[]> {
    const numPages = Math.min(doc.numPages, maxPages || doc.numPages);

    const pages: PageData[] = [];

    // Parse target pages if specified
    let pageNumbers: number[];
    if (targetPages) {
      pageNumbers = this.parseTargetPages(targetPages, doc.numPages);
    } else {
      pageNumbers = Array.from({ length: numPages }, (_, i) => i + 1);
    }

    for (const pageNum of pageNumbers) {
      if (maxPages && pages.length >= maxPages) {
        break;
      }
      const pageData = await this.extractPage(doc, pageNum, options);
      pages.push(pageData);
    }

    return pages;
  }

  async renderPageImage(
    _doc: PdfDocument,
    pageNum: number,
    dpi: number,
    password?: string
  ): Promise<Buffer> {
    const pdfInput = this.currentPdfPath || this.currentPdfData;
    if (!pdfInput) {
      throw new Error("No PDF path or data available for rendering");
    }

    if (!this.pdfiumRenderer) {
      this.pdfiumRenderer = new PdfiumRenderer();
      await this.pdfiumRenderer.loadDocument(pdfInput, password);
    }

    return await this.pdfiumRenderer.renderPageToBuffer(pdfInput, pageNum, dpi, password);
  }

  async close(doc: PdfDocument): Promise<void> {
    const pdfDocument = (doc as PdfJsExtendedDocument)._pdfDocument;
    if (pdfDocument && pdfDocument.destroy) {
      await pdfDocument.destroy();
    }

    // Clean up PDFium renderer (only if it was initialized)
    if (this.pdfiumRenderer) {
      await this.pdfiumRenderer.close();
      this.pdfiumRenderer = null;
    }
    this.currentPdfPath = null;
    this.currentPdfData = null;
  }

  private parseTargetPages(targetPages: string, maxPages: number): number[] {
    const pages: number[] = [];
    const parts = targetPages.split(",");

    for (const part of parts) {
      const trimmed = part.trim();
      if (trimmed.includes("-")) {
        // Range: "1-5"
        const [start, end] = trimmed.split("-").map((n) => parseInt(n.trim()));
        for (let i = start; i <= Math.min(end, maxPages); i++) {
          if (i >= 1) {
            pages.push(i);
          }
        }
      } else {
        // Single page: "10"
        const pageNum = parseInt(trimmed);
        if (pageNum >= 1 && pageNum <= maxPages) {
          pages.push(pageNum);
        }
      }
    }

    return [...new Set(pages)].sort((a, b) => a - b);
  }
}
