// Minimal path stub for browser bundle.
export function dirname(p: string): string {
  const i = p.lastIndexOf("/");
  return i < 0 ? "." : p.slice(0, i) || "/";
}
export function basename(p: string): string {
  const i = p.lastIndexOf("/");
  return i < 0 ? p : p.slice(i + 1);
}
export function join(...parts: string[]): string {
  return parts.filter(Boolean).join("/").replace(/\/+/g, "/");
}
export function resolve(...parts: string[]): string {
  return join(...parts);
}
export function extname(p: string): string {
  const i = p.lastIndexOf(".");
  return i < 0 ? "" : p.slice(i);
}
export const sep = "/";
export default { dirname, basename, join, resolve, extname, sep };
