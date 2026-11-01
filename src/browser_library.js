// IndexedDB is the browser's private library, not permission to modify disk files.
const LIMIT = 32 * 1024 * 1024;
const ROOT = '/Queries';
function validName(name) {
  if (typeof name !== 'string' || !name.trim() || ['.', '..'].includes(name.trim()) || /[\\/:\x00-\x1f]/.test(name)) throw new Error('Choose a name without path separators or control characters.');
  return name.trim();
}
function pathInside(path) {
  if (typeof path !== 'string' || !path.startsWith(ROOT + '/')) throw new Error('Choose a query inside the browser library.');
  path.slice(ROOT.length + 1).split('/').forEach(validName);
  return path;
}
// Pure transaction preparation: no mutation of the supplied snapshot on failure.
export function prepareChange(snapshot, request) {
  const rows = snapshot.map(row => ({...row}));
  let path;
  if (request.op === 'group') {
    for (let n = 1; n <= 5000; n++) {
      path = `${ROOT}/New group${n === 1 ? '' : ' ' + n}`;
      if (!rows.some(row => row.path === path)) break;
    }
    if (rows.some(row => row.path === path)) throw new Error('Too many groups.');
    rows.push({path, kind: 'group'});
  } else if (request.op === 'rename') {
    const old = pathInside(request.path);
    const source = rows.find(row => row.path === old);
    if (!source) throw new Error('The query or group no longer exists.');
    let name = validName(request.name);
    if (source.kind === 'sql') name = validName(name.replace(/\.sql$/i, '')) + '.sql';
    path = old.slice(0, old.lastIndexOf('/') + 1) + name;
    const moving = row => row.path === old || (source.kind === 'group' && row.path.startsWith(old + '/'));
    const occupied = new Set(rows.filter(row => !moving(row)).map(row => row.path));
    // Check every destination before changing paths. A missing group record
    // does not make its surviving child records safe to overwrite.
    for (const row of rows) if (moving(row) && occupied.has(path + row.path.slice(old.length))) throw new Error('That name already exists.');
    for (const row of rows) if (moving(row)) row.path = path + row.path.slice(old.length);
  } else if (request.op === 'save') {
    path = pathInside(request.path);
    if (!path.endsWith('.sql')) throw new Error('Saved queries must use a .sql filename.');
    if (typeof request.sql !== 'string' || request.sql.includes('\0') || new TextEncoder().encode(request.sql).length > 8 * 1024 * 1024) throw new Error('SQL files must be text and at most 8 MiB.');
    const parent = path.slice(0, path.lastIndexOf('/'));
    if (parent !== ROOT && !rows.some(row => row.path === parent && row.kind === 'group')) throw new Error('The selected group no longer exists.');
    const index = rows.findIndex(row => row.path === path);
    if (index >= 0 && (rows[index].kind !== 'sql' || request.expected !== rows[index].sql)) throw new Error('This query changed elsewhere. Reopen it or save under another name.');
    if (index < 0 && request.expected != null) throw new Error('This query was moved or removed. Save under another name.');
    const row = {path, kind: 'sql', sql: request.sql};
    if (index < 0) rows.push(row); else rows[index] = row;
  } else { throw new Error('Unknown library operation.'); }
  if (rows.length > 5000 || new TextEncoder().encode(JSON.stringify(rows)).length > LIMIT) throw new Error('Browser query library exceeds its 5,000-item or 32 MiB limit.');
  const before = new Map(snapshot.map(row => [row.path, row]));
  const after = new Set(rows.map(row => row.path));
  const deletes = snapshot.filter(row => !after.has(row.path)).map(row => row.path);
  const puts = rows.filter(row => {
    const previous = before.get(row.path);
    return !previous || previous.kind !== row.kind || previous.sql !== row.sql;
  });
  return {rows, path, deletes, puts};
}
function openLibrary() {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open('rustable-query-library', 2);
    request.onupgradeneeded = () => {
      const store = request.result.objectStoreNames.contains('entries')
        ? request.transaction.objectStore('entries')
        : request.result.createObjectStore('entries', {keyPath: 'path'});
      if (!store.indexNames.contains('kind')) store.createIndex('kind', 'kind');
    };
    request.onerror = () => reject(request.error);
    request.onsuccess = () => resolve(request.result);
    request.onblocked = () => reject(new Error('Close other demo tabs to upgrade query storage.'));
  });
}
export async function library_request(json) {
  const request = JSON.parse(json);
  const db = await openLibrary();
  try {
    return await new Promise((resolve, reject) => {
      const readOnly = ['list', 'read'].includes(request.op);
      const tx = db.transaction('entries', readOnly ? 'readonly' : 'readwrite');
      const store = tx.objectStore('entries');
      let result, failure;
      tx.oncomplete = () => resolve(JSON.stringify(result));
      tx.onabort = () => reject(failure || tx.error || new Error('Query storage transaction cancelled.'));
      tx.onerror = () => {}; // onabort owns the final failure.
      if (request.op === 'list') {
        // Sidebar refresh reads index keys only, never every SQL document body.
        result = [];
        const cursor = store.index('kind').openKeyCursor();
        cursor.onsuccess = () => {
          const row = cursor.result;
          if (row) { result.push({path: row.primaryKey, kind: row.key}); row.continue(); }
        };
        return;
      }
      const read = request.op === 'read' ? store.get(pathInside(request.path)) : store.getAll();
      read.onsuccess = () => {
        try {
          if (request.op === 'read') {
            if (!read.result || read.result.kind !== 'sql') throw new Error('Saved query no longer exists.');
            result = read.result;
          } else if (request.op === 'list') {
            result = read.result.map(({path, kind}) => ({path, kind}));
          } else {
            const changed = prepareChange(read.result, request);
            for (const path of changed.deletes) store.delete(path);
            for (const row of changed.puts) store.put(row);
            result = {path: changed.path};
          }
        } catch (error) { failure = error; tx.abort(); }
      };
    });
  } finally { db.close(); }
}
