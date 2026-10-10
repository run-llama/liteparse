// The linked WASM binary has WASI imports. The OCR stage tests do not use
// these functions. Fail if a test starts to depend on WASI access.
for (const name of [
  "fd_close",
  "fd_fdstat_get",
  "fd_prestat_get",
  "fd_prestat_dir_name",
  "fd_seek",
  "fd_write",
  "proc_exit",
]) {
  exports[name] = () => {
    throw new Error(`OCR stage test called WASI function ${name}`);
  };
}
