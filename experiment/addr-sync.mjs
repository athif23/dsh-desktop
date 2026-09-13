const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const plane = (path, body) => fetch(`http://127.0.0.1:45331${path}`, {
  method: body === undefined ? 'GET' : 'POST',
  headers: { 'content-type': 'application/json' },
  body: body === undefined ? undefined : JSON.stringify(body),
}).then((r) => r.json())
const dshEval = (js) => plane('/dsh-eval', { js }).then((r) => JSON.parse(r.result ?? 'null'))

await plane('/navigate', { url: 'https://example.com/' })
await sleep(6000)
const info = await dshEval(`(() => {
  const inputs = Array.from(document.querySelectorAll('input'));
  const addr = inputs.find((e) => (e.getAttribute('aria-label') || '').length < 10 && e.value && e.value.startsWith('http'));
  const back = document.querySelector('button[aria-label="Back"]');
  const fwd = document.querySelector('button[aria-label="Forward"]');
  const pick = document.querySelector('button[aria-label="Pick element"]');
  return JSON.stringify({
    addr: addr ? addr.value : null,
    backDisabled: back ? back.disabled : null,
    fwdDisabled: fwd ? fwd.disabled : null,
    backTitle: back ? back.title : null,
    pickDisabled: pick ? pick.disabled : null,
  });
})()`)
console.log('toolbar state:', info)
console.log('ADDR SYNC DONE')
