// Browser stub for file-type. Only PDF magic-byte detection is needed.
export async function fileTypeFromBuffer(buf: Uint8Array) {
  if (buf[0] === 0x25 && buf[1] === 0x50 && buf[2] === 0x44 && buf[3] === 0x46) {
    return { ext: "pdf", mime: "application/pdf" };
  }
  return undefined;
}
export async function fileTypeFromFile() {
  return undefined;
}
