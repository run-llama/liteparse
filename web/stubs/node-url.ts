// Minimal shims for node:url that might get imported into the browser bundle.
export function fileURLToPath(u: string | URL): string {
  return typeof u === "string" ? u : u.href;
}
export function pathToFileURL(p: string): URL {
  return new URL(p, "file://");
}
export const URL = globalThis.URL;
