try {
  // Keep module-fetch failures inside the same fallback as WASM startup failures.
  const {default: init, start} = await import('./pkg/rustable_browser.js');
  await init();
  await start(document.getElementById('rustable'));
  document.getElementById('loading').remove();
  parent.postMessage({type: 'rustable-demo-ready'}, location.origin);
} catch (error) {
  const loading = document.getElementById('loading');
  loading.replaceChildren();
  const message = document.createElement('p');
  message.textContent = 'The WASM demo could not start in this browser.';
  const screenshot = document.createElement('a');
  screenshot.href = '/assets/table-view.png';
  screenshot.textContent = 'See the desktop screenshot instead';
  loading.append(message, screenshot);
  parent.postMessage({type: 'rustable-demo-failed'}, location.origin);
  console.error(error);
}
