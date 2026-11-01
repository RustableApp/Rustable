export function database_cancelled() {
  return globalThis.rustableCancel ? Atomics.load(globalThis.rustableCancel, 0) !== 0 : false;
}
