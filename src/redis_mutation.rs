//! Explicit, bounded mutations. One atomic EVAL exchange; never retry writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Create,
    Add,
    Update,
    Remove,
    Rename,
    DeleteKeys,
    ExpireKeys,
}
#[derive(Clone, Debug)]
pub(crate) struct Mutation {
    pub action: Action,
    pub entries: Vec<(Vec<u8>, Vec<u8>)>,
    pub keys: Vec<Vec<u8>>,
    pub kind: String,
    pub field: Vec<u8>,
    pub value: Vec<u8>,
    pub expected: Vec<u8>,
    pub length: u64,
    pub ttl_ms: Option<u64>,
}
impl Mutation {
    pub fn validate(&self, read_only: bool) -> Result<(), String> {
        if read_only {
            return Err("Unlock editing before changing Redis data.".into());
        }
        if !self.entries.is_empty() {
            if self.action != Action::Create || !matches!(self.kind.as_str(), "hash" | "list" | "set" | "zset") {
                return Err("Multiple entries are supported when creating hashes, lists, sets or sorted sets.".into());
            }
            if self.entries.len() >= 100 { return Err("Create at most 100 entries at once.".into()); }
            let mut total = self.field.len().saturating_add(self.value.len());
            let mut names = std::collections::HashSet::new();
            if self.kind != "list" { names.insert(if self.kind == "set" { self.value.as_slice() } else { self.field.as_slice() }); }
            for (field, value) in &self.entries {
                total = total.saturating_add(field.len()).saturating_add(value.len());
                if total > 65536 { return Err("Combined creation entries are limited to 64 KiB.".into()); }
                if self.kind != "list" && !names.insert(if self.kind == "set" { value.as_slice() } else { field.as_slice() }) {
                    return Err("Creation entries must not repeat a field or member.".into());
                }
                if self.kind == "zset" && !std::str::from_utf8(value).ok().and_then(|s|s.parse::<f64>().ok()).is_some_and(f64::is_finite) {
                    return Err("Every score must be a finite number.".into());
                }
            }
        }
        if self.keys.is_empty()
            || self.keys.len() > 1000
            || self.keys.iter().map(Vec::len).sum::<usize>() > 1024 * 1024
        {
            return Err("Choose between 1 and 1,000 keys (at most 1 MiB of names).".into());
        }
        if !matches!(self.action, Action::DeleteKeys | Action::ExpireKeys) && self.keys.len() != 1 {
            return Err("This operation requires one key.".into());
        }
        if self.field.len() > 65536 || self.value.len() > 65536 || self.expected.len() > 65536 {
            return Err("Editor fields are limited to 64 KiB.".into());
        }
        if self
            .ttl_ms
            .is_some_and(|ttl| ttl == 0 || ttl > i64::MAX as u64)
        {
            return Err("Expiry must be a positive number of milliseconds.".into());
        }
        if matches!(
            self.action,
            Action::DeleteKeys | Action::ExpireKeys | Action::Rename
        ) {
            return Ok(());
        }
        if !matches!(
            self.kind.as_str(),
            "string" | "hash" | "list" | "set" | "zset" | "stream" | "ReJSON-RL"
        ) {
            return Err("Unsupported key type.".into());
        }
        if self.kind == "zset" {
            let score = std::str::from_utf8(&self.value)
                .ok()
                .and_then(|s| s.parse::<f64>().ok());
            if self.action != Action::Remove && !score.is_some_and(f64::is_finite) {
                return Err("Score must be a finite number.".into());
            }
        }
        if self.kind == "ReJSON-RL" && matches!(self.action, Action::Update | Action::Remove) {
            crate::redis_json_path::validate(&self.field, self.action == Action::Remove)?;
        }
        if self.kind == "ReJSON-RL" && self.action != Action::Remove {
            use serde::Deserialize;
            let mut parser = serde_json::Deserializer::from_slice(&self.value);
            serde::de::IgnoredAny::deserialize(&mut parser)
                .map_err(|e| format!("Invalid JSON: {e}"))?;
            parser.end().map_err(|e| format!("Invalid JSON: {e}"))?;
        }
        if self.kind == "list" && matches!(self.action, Action::Update | Action::Remove) {
            let index = std::str::from_utf8(&self.field)
                .ok()
                .and_then(|s| s.parse::<u64>().ok());
            if !index.is_some_and(|n| n < self.length) {
                return Err("List index is out of range.".into());
            }
            if self.action == Action::Remove && self.length > 1000 {
                return Err("Removing by index is limited to lists of 1,000 entries to keep Redis responsive. Use list commands for larger lists.".into());
            }
        }
        if self.kind == "stream" && self.action == Action::Update {
            return Err("Stream entries are immutable; append a new entry instead.".into());
        }
        Ok(())
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub fn action_name(&self) -> &'static str {
        match self.action {
            Action::Create => "create",
            Action::Add => "add",
            Action::Update => "update",
            Action::Remove => "remove",
            Action::Rename => "rename",
            Action::DeleteKeys => "delete",
            Action::ExpireKeys => "expire",
        }
    }
}
// Validate all preconditions before the first mutating command. Redis Lua errors
// do not roll back prior writes. No user-provided Lua is evaluated.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const SCRIPT: &str = r#"
local op,kind,field,value,expected=ARGV[1],ARGV[2],ARGV[3],ARGV[4],ARGV[5]
local length=tonumber(ARGV[6]); local ttl=ARGV[7]
if op=='delete' then return redis.call('UNLINK',unpack(KEYS)) end
if op=='expire' then
 for _,key in ipairs(KEYS) do
  local allowed=false; if ttl=='' then allowed=redis.acl_check_cmd('PERSIST',key) else allowed=redis.acl_check_cmd('PEXPIRE',key,ttl) end
  if not allowed then return redis.error_reply('Expiry permission denied; no keys changed.') end
 end
 local n=0; for _,key in ipairs(KEYS) do
  if ttl=='' then n=n+redis.call('PERSIST',key) else n=n+redis.call('PEXPIRE',key,ttl) end
 end; return n
end
local key=KEYS[1]; local actual=redis.call('TYPE',key).ok
if op=='create' then
 if actual~='none' then return redis.error_reply('Key already exists; choose another name.') end
 if ttl~='' and not redis.acl_check_cmd('PEXPIRE',key,ttl) then return redis.error_reply('Expiry permission denied; key was not created.') end
 if kind=='string' then redis.call('SET',key,value)
 elseif kind=='hash' or kind=='list' or kind=='set' or kind=='zset' then
  local args={key}; local count=tonumber(ARGV[8]) or 0
  local function append(f,v)
   if kind=='hash' then args[#args+1]=f; args[#args+1]=v
   elseif kind=='zset' then args[#args+1]=v; args[#args+1]=f
   else args[#args+1]=v end
  end
  append(field,value)
  for i=1,count do append(ARGV[8+i*2-1],ARGV[8+i*2]) end
  local command=({hash='HSET',list='RPUSH',set='SADD',zset='ZADD'})[kind]
  redis.call(command,unpack(args))
 elseif kind=='stream' then redis.call('XADD',key,'*',field,value)
 elseif kind=='ReJSON-RL' then redis.call('JSON.SET',key,'$',value)
 else return redis.error_reply('Unsupported key type') end
 if ttl~='' then redis.call('PEXPIRE',key,ttl) end
 return 1
end
if actual~=kind then return redis.error_reply('Key expired or changed type. Refresh before editing.') end
if op=='rename' then
 local destination=KEYS[2]
 if redis.call('EXISTS',destination)~=0 then return redis.error_reply('Destination key already exists.') end
 return redis.call('RENAMENX',key,destination)
end
if kind=='hash' then
 if op=='add' then
  if redis.call('HEXISTS',key,field)~=0 then return redis.error_reply('Field already exists.') end
  redis.call('HSET',key,field,value)
 else
  if redis.call('HGET',key,field)~=expected then return redis.error_reply('Field changed. Refresh before editing.') end
  if op=='remove' then redis.call('HDEL',key,field) else redis.call('HSET',key,field,value) end
 end
elseif kind=='list' then
 if op=='add' then redis.call('RPUSH',key,value)
 else
  if redis.call('LLEN',key)~=length or redis.call('LINDEX',key,field)~=expected then return redis.error_reply('List changed. Refresh before editing.') end
  if op=='update' then redis.call('LSET',key,field,value)
  else
   if length>1000 then return redis.error_reply('List is too large for removal by index.') end
   if not redis.acl_check_cmd('LSET',key,field,value) or not redis.acl_check_cmd('RPOP',key) then return redis.error_reply('List edit permission denied.') end
   for i=tonumber(field),length-2 do redis.call('LSET',key,i,redis.call('LINDEX',key,i+1)) end
   redis.call('RPOP',key)
  end
 end
elseif kind=='set' then
 if op=='add' then
  if redis.call('SISMEMBER',key,value)~=0 then return redis.error_reply('Member already exists.') end
  redis.call('SADD',key,value)
 else
  if redis.call('SISMEMBER',key,field)==0 then return redis.error_reply('Member disappeared. Refresh before editing.') end
  if op=='remove' then redis.call('SREM',key,field)
  elseif value~=field then
   if redis.call('SISMEMBER',key,value)~=0 then return redis.error_reply('Replacement member already exists.') end
   if not redis.acl_check_cmd('SADD',key,value) or not redis.acl_check_cmd('SREM',key,field) then return redis.error_reply('Member edit permission denied.') end
   redis.call('SADD',key,value); redis.call('SREM',key,field)
  end
 end
elseif kind=='zset' then
 local old=redis.call('ZSCORE',key,field)
 if op=='add' then
  if old then return redis.error_reply('Member already exists.') end
  redis.call('ZADD',key,value,field)
 else
  if not old or tonumber(old)~=tonumber(expected) then return redis.error_reply('Score changed. Refresh before editing.') end
  if op=='remove' then redis.call('ZREM',key,field) else redis.call('ZADD',key,value,field) end
 end
elseif kind=='stream' then
 if op=='add' then redis.call('XADD',key,'*',field,value)
 elseif op=='remove' then
  if redis.call('XDEL',key,field)==0 then return redis.error_reply('Entry disappeared. Refresh before editing.') end
 else return redis.error_reply('Stream entries are immutable.') end
elseif kind=='ReJSON-RL' and (op=='update' or op=='remove') then
 if redis.call('JSON.GET',key)~=expected then return redis.error_reply('JSON changed. Refresh before editing.') end
 local path=field; if path=='' then path='$' end
 if op=='remove' then
  if path=='$' then return redis.error_reply('Use Delete key to remove the whole document.') end
  if redis.call('JSON.DEL',key,path)==0 then return redis.error_reply('JSON path does not exist; nothing changed.') end
 else
  if not redis.call('JSON.SET',key,path,value) then return redis.error_reply('JSON parent path does not exist; nothing changed.') end
 end
else return redis.error_reply('Unsupported edit for this key type.') end
return 1
"#;
