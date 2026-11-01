// Separate from WASM so database/background workers never load the CJK font.
const SIZE = 16467736;
const SHA256 = '68a3fc98800b2a27b371f2fb79991daf3633bd89309d4ffaa6946fd587f375b5';

export async function read_cjk_font(response) {
  if (!response.ok || !response.body) throw new Error('CJK font download failed.');
  const reader = response.body.getReader();
  const bytes = new Uint8Array(SIZE);
  let offset = 0;
  try {
    while (true) {
      const {done, value} = await reader.read();
      if (done) break;
      if (value.length > SIZE - offset) throw new Error('CJK font exceeds its size limit.');
      bytes.set(value, offset);
      offset += value.length;
    }
    if (offset !== SIZE) throw new Error('CJK font download is incomplete.');
    const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', bytes));
    const hash = Array.from(digest, byte => byte.toString(16).padStart(2, '0')).join('');
    if (hash !== SHA256) throw new Error('CJK font integrity check failed.');
    return bytes;
  } finally {
    await reader.cancel();
    reader.releaseLock();
  }
}

export async function load_cjk_font() {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), 30000);
  try {
    return await read_cjk_font(await fetch('./fonts/NotoSansCJKjp-Regular.otf', {signal: controller.signal}));
  } catch (error) {
    console.warn('Rustable CJK font unavailable; retaining default fonts.', error);
    throw error;
  } finally {
    clearTimeout(timeout);
  }
}
