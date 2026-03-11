export async function importPdfJs() {
  const pdfUrl = new URL("../../vendor/pdfjs/pdf.mjs", import.meta.url);
  const pdfjs = await import(pdfUrl.href);

  return {
    fn: pdfjs.getDocument,
    dir: pdfUrl
  };
}