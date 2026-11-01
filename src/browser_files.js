// Only a user-selected file is read. The picker never grants directory access.
export function pick_sql() { return pick_file('.sql,text/plain,application/sql', 'SQL editor'); }
export function pick_database() { return pick_file('.sqlite,.sqlite3,.db,application/vnd.sqlite3', 'SQLite'); }
export function pick_csv() { return pick_file('.csv,text/csv', 'CSV'); }
function pick_file(accept, label) {
  return new Promise((resolve, reject) => {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = accept;
    input.hidden = true;
    const clean = () => input.remove();
    input.addEventListener('cancel', () => { clean(); resolve(null); }, {once: true});
    input.addEventListener('change', async () => {
      try {
        const file = input.files[0];
        if (!file) { resolve(null); return; }
        if (file.size > 8 * 1024 * 1024) throw new Error(`${label} files are limited to 8 MiB`);
        resolve({name: file.name, bytes: new Uint8Array(await file.arrayBuffer())});
      } catch (error) { reject(error); }
      finally { clean(); }
    }, {once: true});
    document.body.append(input);
    input.click();
  });
}
export function download_sql(name, sql) {
  download_bytes(name, new TextEncoder().encode(sql), 'application/sql;charset=utf-8');
}
export function download_bytes(name, bytes, mime) {
  const url = URL.createObjectURL(new Blob([bytes], {type: mime}));
  const link = document.createElement('a');
  link.href = url;
  link.download = name.replace(/[\\/]/g, '_');
  document.body.append(link);
  try { link.click(); }
  finally { link.remove(); setTimeout(() => URL.revokeObjectURL(url), 60000); }
}
