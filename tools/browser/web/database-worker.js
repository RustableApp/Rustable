import init, {database_command} from './pkg/rustable_browser.js';
let queue = init();
self.onmessage = ({data}) => {
  if (data.cancel) {
    globalThis.rustableCancel = new Int32Array(data.cancel);
    return;
  }
  queue = queue.then(() => {
    const result = database_command(data);
    self.postMessage(result, [result.buffer]);
  }).catch(error => { setTimeout(() => { throw error; }); });
};
