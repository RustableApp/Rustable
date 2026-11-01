import init, {background_job} from './pkg/rustable_browser.js';
const ready = init();
self.onmessage = async ({data}) => {
  try {
    await ready;
    const result = background_job(data);
    self.postMessage(result, [result.buffer]);
  } catch (error) { setTimeout(() => { throw error; }); }
};
