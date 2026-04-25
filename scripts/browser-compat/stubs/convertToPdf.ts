// Browser stub — only PDF input is supported, no file conversion.
export const officeExtensions: string[] = [];
export const spreadsheetExtensions: string[] = [];
export const imageExtensions: string[] = [];
export const htmlExtensions: string[] = [];
export async function convertToPdf() {
  throw new Error("File conversion is not supported in browser environments.");
}
export async function convertBufferToPdf() {
  throw new Error("File conversion is not supported in browser environments.");
}
export async function cleanupConversionFiles() {}
export async function guessExtensionFromBuffer(data: Uint8Array) {
  if (data[0] === 0x25 && data[1] === 0x50 && data[2] === 0x44 && data[3] === 0x46) {
    return ".pdf";
  }
  return null;
}
