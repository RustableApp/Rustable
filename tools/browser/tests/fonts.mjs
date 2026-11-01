import {readFile} from 'node:fs/promises';
import assert from 'node:assert/strict';
import test from 'node:test';
const source = await readFile(new URL('../../../src/browser_fonts.js', import.meta.url), 'utf8');
const {read_cjk_font} = await import('data:text/javascript;base64,' + Buffer.from(source).toString('base64'));
const font = await readFile(new URL('../../../assets/fonts/NotoSansCJKjp-Regular.otf', import.meta.url));
test('accept the pinned CJK asset', async () => {
  assert.deepEqual(await read_cjk_font(new Response(font)), new Uint8Array(font));
});
test('reject HTTP errors, truncated and same-size corrupt font data', async () => {
  await assert.rejects(read_cjk_font(new Response('', {status:404})), /download failed/);
  await assert.rejects(read_cjk_font(new Response(font.subarray(0,100))), /incomplete/);
  const corrupt = Buffer.from(font);
  corrupt[100] ^= 1;
  await assert.rejects(read_cjk_font(new Response(corrupt)), /integrity/);
});
test('stop and cancel a response that exceeds the font byte budget', async () => {
  let cancelled = false;
  const stream = new ReadableStream({
    start(controller) { controller.enqueue(new Uint8Array(font.length + 1)); },
    cancel() { cancelled = true; },
  });
  await assert.rejects(read_cjk_font(new Response(stream)), /size limit/);
  assert.equal(cancelled, true);
});
