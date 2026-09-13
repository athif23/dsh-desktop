const plane = (path, body) => fetch(`http://127.0.0.1:45331${path}`, {
  method: body === undefined ? 'GET' : 'POST',
  headers: { 'content-type': 'application/json' },
  body: body === undefined ? undefined : JSON.stringify(body),
}).then((r) => r.json())
const dshEval = (js) => plane('/dsh-eval', { js }).then((r) => JSON.parse(r.result ?? 'null'))

const readout = await dshEval(`(() => {
  const spans = Array.from(document.querySelectorAll('span'));
  const hits = spans.map((e) => e.textContent).filter((s) => s && s.indexOf(' \\u00b7 ') >= 0);
  return JSON.stringify(hits.map((s) => s.slice(0, 160)).slice(0, 6));
})()`)
console.log('readout lines:', readout)
console.log('READOUT2 DONE')
