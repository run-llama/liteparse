// Browser stub for the convertToPdf module. We only accept PDF bytes;
// anything else errors, which LiteParse surfaces to the UI.

import { fileTypeFromBuffer } from "./file-type.js";

export async function convertToPdf(): Promise<never> {
  throw new Error("File path conversion is not supported in the browser — pass PDF bytes.");
}

export async function convertBufferToPdf(): Promise<never> {
  throw new Error("Non-PDF formats are not supported in the browser build.");
}

export async function cleanupConversionFiles(): Promise<void> {
  // no-op in browser
}

export async function guessExtensionFromBuffer(buf: Uint8Array | ArrayBuffer): Promise<string> {
  const t = await fileTypeFromBuffer(buf);
  if (t?.ext === "pdf") return ".pdf";
  throw new Error("Only PDF files are supported in the browser build.");
}

export const officeExtensions: string[] = [];
export const spreadsheetExtensions: string[] = [];
export const imageExtensions: string[] = [];

export function getTmpDir(): string {
  return "/tmp";
}
