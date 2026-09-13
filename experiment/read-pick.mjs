const plane = (path, body) => fetch(`http://127.0.0.1:45331${path}`, {
  method: body === undefined ? 'GET' : 'POST',
  headers: { 'content-type': 'application/json' },
  body: body === undefined ? undefined : JSON.stringify(body),
}).then((r) => r.json())
const dshEval = (js) => plane('/dsh-eval', { js }).then((r) => JSON.parse(r.result ?? 'null'))

const readout = await dshEval(`(() => {
  const spans = Array.from(document.querySelectorAll('span'));
  const line = spans.map((e) => e.textContent).find((s) => s && s.includes(' · ') && (s.includes('#') || s.includes(' > ')));
  const pickBtn = document.querySelector('button[aria-label="Pick element"]');
  return JSON.stringify({ readout: (line || '').slice(0, 220), pickDisabled: pickBtn ? pickBtn.disabled : null });
})()`)
console.log('pick readout:', readout)
console.log('READOUT DONE')
