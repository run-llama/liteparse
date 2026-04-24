// Empty stub for Node-only modules in the browser bundle.
// Any access throws — signals we accidentally hit a Node-only code path.
const handler: ProxyHandler<object> = {
  get(_t, prop) {
    if (prop === "__esModule") return true;
    if (prop === Symbol.toPrimitive || prop === Symbol.toStringTag) return undefined;
    throw new Error(`[liteparse-browser] accessed stubbed Node module property: ${String(prop)}`);
  },
};
const stub = new Proxy({}, handler);
export default stub;
export const promises = stub;
export const constants = stub;
export const createReadStream = () => {
  throw new Error("createReadStream is not available in the browser");
};
export const readFile = () => {
  throw new Error("fs.readFile is not available in the browser");
};
export const spawn = () => {
  throw new Error("child_process.spawn is not available in the browser");
};
