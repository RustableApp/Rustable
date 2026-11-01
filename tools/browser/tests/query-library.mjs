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

test('saving one document writes only that document and preserves unrelated entries', () => {
  const rows = [{path:'/Queries/a.sql',kind:'sql',sql:'SELECT 1;'},
    {path:'/Queries/b.sql',kind:'sql',sql:'SELECT 2;'}, {path:'/Queries/Empty',kind:'group'}];
  const changed = prepareChange(rows, {op:'save',path:'/Queries/a.sql',sql:'SELECT 3;',expected:'SELECT 1;'});
  assert.deepEqual(changed.deletes, []);
  assert.deepEqual(changed.puts, [{path:'/Queries/a.sql',kind:'sql',sql:'SELECT 3;'}]);
  const unchanged = prepareChange(rows, {op:'save',path:'/Queries/a.sql',sql:'SELECT 1;',expected:'SELECT 1;'});
  assert.deepEqual(unchanged.puts, []);
});
test('group rename deletes only old paths and writes only moved entries', () => {
  const rows = [{path:'/Queries/A',kind:'group'}, {path:'/Queries/A/q.sql',kind:'sql',sql:'SELECT 1;'},
    {path:'/Queries/AB/q.sql',kind:'sql',sql:'SELECT 2;'}];
  const changed = prepareChange(rows, {op:'rename',path:'/Queries/A',name:'C'});
  assert.deepEqual(changed.deletes, ['/Queries/A','/Queries/A/q.sql']);
  assert.deepEqual(changed.puts, [{path:'/Queries/C',kind:'group'}, {path:'/Queries/C/q.sql',kind:'sql',sql:'SELECT 1;'}]);
  const applied = new Map(rows.map(row => [row.path,row]));
  changed.deletes.forEach(path => applied.delete(path));
  changed.puts.forEach(row => applied.set(row.path,row));
  assert.deepEqual([...applied.values()].sort((a,b)=>a.path.localeCompare(b.path)), changed.rows.sort((a,b)=>a.path.localeCompare(b.path)));
});
