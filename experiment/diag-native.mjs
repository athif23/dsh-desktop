const plane = (path, body) => fetch(`http://127.0.0.1:45331${path}`, {
  method: body === undefined ? 'GET' : 'POST',
  headers: { 'content-type': 'application/json' },
  body: body === undefined ? undefined : JSON.stringify(body),
}).then((r) => r.json())
const dshEval = (js) => plane('/dsh-eval', { js }).then((r) => JSON.parse(r.result ?? 'null'))

// Frame-server view (what the toolbar polls).
const status = await fetch('http://127.0.0.1:19333/status').then((r) => r.json()).catch((e) => ({ error: String(e) }))
console.log('frame /status native:', JSON.stringify(status.native ?? status).slice(0, 300))

// Panel DOM: status line + pick buttons (all of them).
const dom = await dshEval(`(() => {
  const all = Array.from(document.body.innerText.split('\\n')).filter((l) => /Live|Loading|Connecting|native|stream/.test(l));
  const picks = Array.from(document.querySelectorAll('button[aria-label="Pick element"]')).map((e) => ({ dis: e.disabled, t: e.title }));
  return JSON.stringify({ statusLines: all.slice(0, 4), picks });
})()`)
console.log('panel:', dom)
console.log('DIAG DONE')
