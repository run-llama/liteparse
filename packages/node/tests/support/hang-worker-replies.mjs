// Preloaded into pool worker processes (through NODE_OPTIONS=--import) by
// pool.test.mjs when a test needs a parse that never finishes. It drops the
// worker's parse replies ("ok" and "err"), so the parent sees a parse that
// outlives any deadline. The init handshake ("ready") still goes through.
// This makes the timeout tests independent of how fast PDFium parses on the
// runner.
const send = process.send?.bind(process);
if (send === undefined) {
  throw new Error("hang-worker-replies.mjs must be preloaded into a forked worker");
}
process.send = (message, ...rest) => {
  if (message && (message.type === "ok" || message.type === "err")) return true;
  return send(message, ...rest);
};
