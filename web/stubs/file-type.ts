// Browser replacement for the `file-type` package. We only care about PDF.
function isPdf(bytes: Uint8Array): boolean {
  return (
    bytes.length >= 4 &&
    bytes[0] === 0x25 && // %
    bytes[1] === 0x50 && // P
    bytes[2] === 0x44 && // D
    bytes[3] === 0x46 // F
  );
}

export async function fileTypeFromBuffer(
  buf: Uint8Array | ArrayBuffer
): Promise<{ ext: string; mime: string } | undefined> {
  const bytes = buf instanceof Uint8Array ? buf : new Uint8Array(buf);
  if (isPdf(bytes)) return { ext: "pdf", mime: "application/pdf" };
  return undefined;
}

export async function fileTypeFromFile(): Promise<undefined> {
  return undefined;
}
