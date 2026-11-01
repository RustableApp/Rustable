import {readFile} from 'node:fs/promises';
import assert from 'node:assert/strict';
import test from 'node:test';
const source = await readFile(new URL('../../../src/browser_library.js', import.meta.url), 'utf8');
const {prepareChange} = await import('data:text/javascript;base64,' + Buffer.from(source).toString('base64'));
test('rename a group moves its SQL children without changing content', () => {
  const original = [{path:'/Queries/A',kind:'group'}, {path:'/Queries/A/q.sql',kind:'sql',sql:"SELECT '日本';"}];
  const changed = prepareChange(original, {op:'rename',path:'/Queries/A',name:'B'});
  assert.equal(changed.rows[1].path, '/Queries/B/q.sql');
  assert.equal(changed.rows[1].sql, original[1].sql);
  assert.equal(original[1].path, '/Queries/A/q.sql');
});
test('save rejects conflicts and missing baselines without changing snapshot', () => {
  const original = [{path:'/Queries/q.sql',kind:'sql',sql:'SELECT 1;'}];
  assert.throws(() => prepareChange(original,{op:'save',path:'/Queries/q.sql',sql:'SELECT 2;'}), /changed elsewhere/);
  assert.throws(() => prepareChange([],{op:'save',path:'/Queries/q.sql',sql:'SELECT 2;',expected:'SELECT 1;'}), /moved or removed/);
  assert.equal(prepareChange(original,{op:'save',path:'/Queries/q.sql',sql:'SELECT 2;',expected:'SELECT 1;'}).rows[0].sql,'SELECT 2;');
  assert.equal(original[0].sql,'SELECT 1;');
});
test('paths, collisions, names and limits are checked before mutation', () => {
  const rows = [{path:'/Queries/A',kind:'group'}, {path:'/Queries/B',kind:'group'}];
  for (const name of ['B','../C','', '.', '..','x/y','x\\y']) assert.throws(() => prepareChange(rows,{op:'rename',path:'/Queries/A',name}));
  assert.throws(() => prepareChange([], {op:'save', path:'/Queries/../q.sql',sql:'SELECT 1'}));
  assert.throws(() => prepareChange([], {op:'save', path:'/Queries/q.sql',sql:'x'.repeat(8*1024*1024+1)}));
  assert.throws(() => prepareChange([], {op:'save', path:'/Queries/missing/q.sql',sql:'SELECT 1'}));
  assert.equal(prepareChange([{path:'/Queries/New group',kind:'group'}],{op:'group'}).path,'/Queries/New group 2');
});
