// Exercise the same worker source against SQLite WASM's Node distribution.
// Run after build-browser.py has staged SQLite, using mise exec node@22 -- node ...
import assert from 'node:assert/strict';
import {readFile,writeFile} from 'node:fs/promises';
const base=new URL('./',import.meta.url);
const source=await readFile(new URL('web/sqlite-worker.js',base),'utf8');
const adapted=source.replace("'./sqlite/index.mjs'",JSON.stringify(new URL('target/sqlite/node.mjs',base).href));
await writeFile(new URL('target/test-worker.mjs',base),adapted);
globalThis.self=globalThis;
const sample=await readFile(new URL('../../src/demo.sql',base),'utf8');
globalThis.fetch=async ()=>({ok:true,text:async()=>sample});
await import(new URL('target/test-worker.mjs',base));
let id=0;
const query=(sql,extra={})=>new Promise(resolve=>{
  const requestId=++id;
  globalThis.postMessage=message=>{assert.equal(message.id,requestId);resolve(message.reply);};
  self.onmessage({data:{id:requestId,sql,caret:0,selectionEnd:0,all:true,...extra}});
});
let r=await query("SELECT 9007199254740993 AS big, NULL AS empty; UPDATE customers SET name='WASM visitor' WHERE id=1; SELECT name FROM customers WHERE id=1; SELECT id FROM customers WHERE id<0;");
assert.equal(r.error,null);assert.equal(r.results.length,4);
assert.deepEqual(r.results[0].rows,[['9007199254740993','NULL']]);
assert.deepEqual(r.results[2].rows,[['WASM visitor']]);
assert.deepEqual(r.results[3].columns,['id']);assert.equal(r.results[3].rows.length,0);
r=await query('SELECT * FROM customers');assert.equal(r.results[0].rows.length,500);assert.equal(r.results[0].truncated,true);
r=await query("SELECT 'first'; SELECT 'second';",{all:false,caret:15,selectionEnd:15});assert.equal(r.results.length,1);assert.deepEqual(r.results[0].rows,[['first']]);
r=await query("SELECT 'é'; SELECT 42;",{all:false,caret:12,selectionEnd:22});assert.deepEqual(r.results[0].rows,[['42']]);
r=await query('SELECT missing_column FROM customers');assert.ok(r.error);
r=await query('SELECT 42');assert.equal(r.error,null);assert.deepEqual(r.results[0].rows,[['42']]);
r=await query('WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n) SELECT sum(x) FROM n');assert.ok(r.error);assert.ok(r.elapsed<5000);
r=await query('SELECT 7');assert.equal(r.error,null);
console.log('SQLite WASM worker: mutations, precise integers, NULL, empty results, row cap, current/selection, error recovery and timeout passed.');
