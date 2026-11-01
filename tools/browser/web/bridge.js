// Browser integration only. UI and rendering live in Rust/WASM; SQLite is WASM.
let worker;
let pending;
let sequence = 0;
export function cancel() {
  worker?.terminate(); worker = undefined;
  if (pending) { clearTimeout(pending.timer); pending.reject('Cancelled. Resetting the browser sandbox.'); pending = undefined; }
}
export function request(sql, caret, selectionEnd, all, reset) {
  if (reset) { cancel(); sql = 'SELECT * FROM customers LIMIT 50;'; all = true; }
  if (pending) return Promise.reject('A query is already running.');
  if (!worker) {
    worker = new Worker('/demo/sqlite-worker.js', {type:'module'});
    worker.onmessage = ({data}) => {
      if (!pending || data.id !== pending.id) return;
      clearTimeout(pending.timer); const resolve = pending.resolve; pending = undefined;
      resolve(JSON.stringify(data.reply));
    };
    worker.onerror = () => cancel();
  }
  return new Promise((resolve,reject) => {
    const id = ++sequence;
    const timer = setTimeout(() => { cancel(); }, 15000);
    pending = {id,resolve,reject,timer};
    worker.postMessage({id,sql,caret,selectionEnd,all});
  });
}
