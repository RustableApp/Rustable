import {readFile} from 'node:fs/promises';
import assert from 'node:assert/strict';
import test from 'node:test';

const source = await readFile(new URL('../../../src/browser_files.js', import.meta.url), 'utf8');
test('download buffers are bounded and released on expiry or launch failure', async () => {
  const original = {document: globalThis.document, setTimeout: globalThis.setTimeout,
    create: URL.createObjectURL, revoke: URL.revokeObjectURL};
  const active = new Set(), timers = [], links = [];
  let sequence = 0, fail = false;
  URL.createObjectURL = blob => { assert.ok(blob instanceof Blob); const id = `blob:${++sequence}`; active.add(id); return id; };
  URL.revokeObjectURL = id => active.delete(id);
  globalThis.setTimeout = (callback, delay) => { assert.equal(delay,60000); timers.push(callback); };
  globalThis.document = {body: {append() {}}, createElement(tag) {
    assert.equal(tag,'a');
    const link = {click() { if (fail) throw new Error('download denied'); }, remove() { this.removed = true; }};
    links.push(link); return link;
  }};
  try {
    const {download_bytes} = await import('data:text/javascript;base64,' + Buffer.from(source).toString('base64'));
    const bytes = new Uint8Array(16 * 1024 * 1024);
    download_bytes('../first.csv',bytes,'text/csv');
    download_bytes('second.csv',bytes,'text/csv');
    assert.equal(active.size,2);
    assert.equal(links[0].download,'.._first.csv');
    assert.ok(links.every(link=>link.removed));
    assert.throws(()=>download_bytes('third.csv',new Uint8Array(1),'text/csv'),/Pending downloads/);
    assert.equal(active.size,2, 'rejection must not allocate a new object URL');
    timers.shift()();
    assert.equal(active.size,1);
    fail = true;
    assert.throws(()=>download_bytes('failed.csv',bytes,'text/csv'),/download denied/);
    assert.equal(active.size,1, 'failed launch releases its Blob immediately');
    fail = false;
    download_bytes('retry.csv',bytes,'text/csv');
    assert.equal(active.size,2, 'failed launch must return its memory budget');
    for (const expire of timers) { expire(); expire(); }
    assert.equal(active.size,0);
    download_bytes('after-expiry.csv',new Uint8Array(32*1024*1024),'text/csv');
    assert.throws(()=>download_bytes('over-budget.csv',new Uint8Array(1),'text/csv'),/Pending downloads/);
    timers.at(-1)();
  } finally {
    globalThis.document = original.document; globalThis.setTimeout = original.setTimeout;
    URL.createObjectURL = original.create; URL.revokeObjectURL = original.revoke;
  }
});
