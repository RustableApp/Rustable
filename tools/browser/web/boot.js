
import init,{start} from './pkg/rustable_browser.js';
try { await init(); await start(document.getElementById('rustable')); document.getElementById('loading').remove(); parent.postMessage({type:'rustable-demo-ready'},location.origin); }
catch(error) { const el=document.getElementById('loading');el.replaceChildren();const p=document.createElement('p');p.textContent='The WASM demo could not start in this browser.';const a=document.createElement('a');a.href='/assets/table-view.png';a.textContent='See the desktop screenshot instead';el.append(p,a);parent.postMessage({type:'rustable-demo-failed'},location.origin);console.error(error); }
