const plane = (path, body) => fetch(`http://127.0.0.1:45331${path}`, {
  method: body === undefined ? 'GET' : 'POST',
  headers: { 'content-type': 'application/json' },
  body: body === undefined ? undefined : JSON.stringify(body),
}).then((r) => r.json())
const dshEval = (js) => plane('/dsh-eval', { js }).then((r) => JSON.parse(r.result ?? 'null'))

const st = await dshEval(`(() => {
  const pick = document.querySelector('button[aria-label="Pick element"]');
  const btns = Array.from(document.querySelectorAll('button')).map((e) => e.getAttribute('aria-label') + '=' + e.disabled).filter((s) => /Back|Forward|Reload|Pick|Go/.test(s));
  return JSON.stringify({ pickTitle: pick ? pick.title : null, pickDis: pick ? pick.disabled : null, btns });
})()`)
console.log('panel buttons:', st)
console.log('BTNS DONE')
