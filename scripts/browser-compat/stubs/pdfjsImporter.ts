// Browser stub for pdfjsImporter.
// In a real browser app, this would configure PDF.js worker and asset URLs.
// @ts-expect-error vendored ESM build has no types
import * as pdfjs from "../../../src/vendor/pdfjs/pdf.mjs";

export async function importPdfJs() {
  return {
    fn: (pdfjs as { getDocument: unknown }).getDocument,
    dir: new URL("../../../src/vendor/pdfjs", import.meta.url).href,
  };
}
