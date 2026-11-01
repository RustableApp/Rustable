import init from './sqlite/index.mjs';
const ready = (async () => {
  const sqlite = await init({print:()=>{},printErr:()=>{}});
  const db = new sqlite.oo1.DB(':memory:', 'c');
  db.exec('PRAGMA foreign_keys=ON; PRAGMA hard_heap_limit=67108864; PRAGMA max_page_count=8192;');
  const c = sqlite.capi;
  c.sqlite3_limit(db.pointer,c.SQLITE_LIMIT_LENGTH,1024*1024);
  c.sqlite3_limit(db.pointer,c.SQLITE_LIMIT_SQL_LENGTH,128*1024);
  c.sqlite3_limit(db.pointer,c.SQLITE_LIMIT_COLUMN,128);
  const response = await fetch('./sample.sql');
  if (!response.ok) throw new Error('Sample database could not be loaded.');
  db.exec(await response.text());
  return {sqlite,db};
})();
function executable(sql) { return sql.replace(/--[^\n]*(?:\n|$)|\/\*[\s\S]*?\*\//g,'').trim().replaceAll(';','').length > 0; }
function current(sql, caret, end, all, capi) {
  if (all) return sql;
  const chars=Array.from(sql);
  if(caret!==end) return chars.slice(caret,end).join('');
  const offset=chars.slice(0,caret).join('').length;
  const ranges=[]; let start=0;
  for(let i=0;i<sql.length;i++) if(sql[i]===';' && capi.sqlite3_complete(sql.slice(start,i+1))) {
    if(executable(sql.slice(start,i+1))) ranges.push([start,i+1]); start=i+1;
  }
  if(executable(sql.slice(start))) ranges.push([start,sql.length]);
  const r=ranges.find(([a,b])=>a<=offset&&offset<=b) ?? ranges.at(-1);
  return r ? sql.slice(...r) : '';
}
self.onmessage=async ({data})=>{
  const reply={results:[],tables:[],elapsed:0,error:null};
  const started=performance.now();
  let state;
  try {
    state=await ready;
    const {sqlite,db}=state, {capi:c,wasm:w}=sqlite;
    if(data.sql.length>128*1024) throw new Error('Demo SQL is limited to 128 KB.');
    const sql=current(data.sql,data.caret,data.selectionEnd,data.all,c);
    const deadline=performance.now()+2000;
    c.sqlite3_progress_handler(db.pointer,1000,()=>performance.now()>deadline ? 1 : 0,0);
    const scope=w.scopedAllocPush();
    try {
      let p=w.scopedAllocCString(sql);
      const [statement,tail]=w.scopedAllocPtr(2);
      let byteCount=0;
      while(w.peek(p,'i8')) {
        const rc=c.sqlite3_prepare_v3(db.pointer,p,-1,0,statement,tail);
        if(rc) throw new Error(c.sqlite3_errmsg(db.pointer));
        const ptr=w.peekPtr(statement), next=w.peekPtr(tail);
        if(next<=p) throw new Error('SQLite could not advance through the script.');
        p=next;
        if(!ptr) continue;
        const stmt=sqlite.oo1.Stmt.wrapHandle(db,ptr,true);
        try {
          if(reply.results.length>=50) throw new Error('Demo scripts are limited to 50 statements.');
          const set={columns:Array.from({length:stmt.columnCount},(_,i)=>c.sqlite3_column_name(ptr,i)),rows:[],truncated:false};
          const readonly=c.sqlite3_stmt_readonly(ptr);
          while(stmt.step()) {
            if(set.rows.length>=500 || byteCount>=1024*1024) { set.truncated=true; if(readonly) break; continue; }
            const row=[];
            for(let i=0;i<stmt.columnCount;i++) {
              const v=stmt.get(i);
              const text=v===null ? 'NULL' : v instanceof Uint8Array ? `BLOB · ${v.length} bytes` : String(v);
              row.push(text.length>4096 ? text.slice(0,4096)+'…' : text);
              byteCount+=row.at(-1).length*2;
            }
            set.rows.push(row);
          }
          reply.results.push(set);
        } finally { stmt.finalize(); }
      }
    } finally { w.scopedAllocPop(scope); c.sqlite3_progress_handler(db.pointer,0,null,0); }
  } catch(error) { reply.error=String(error.message??error); }
  if(state) {
    try { reply.tables=state.db.selectValues("SELECT name FROM sqlite_schema WHERE type IN ('table','view') AND name NOT LIKE 'sqlite_%' ORDER BY name LIMIT 200"); }
    catch { /* Keep the original query error. Reset remains available. */ }
  }
  reply.elapsed=performance.now()-started;
  postMessage({id:data.id,reply});
};
