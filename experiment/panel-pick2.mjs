/** Panel pick without the race: press, wait for armed flag, click, read. */
import { NativeSurface } from '../../dsh-browser/src/native.js'

const n = new NativeSurface('http://127.0.0.1:45331')
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const plane = (path, body) => fetch(`http://127.0.0.1:45331${path}`, {
  method: body === undefined ? 'GET' : 'POST',
  headers: { 'content-type': 'application/json' },
  body: body === undefined ? undefined : JSON.stringify(body),
}).then((r) => r.json())
const dshEval = (js) => plane('/dsh-eval', { js }).then((r) => JSON.parse(r.result ?? 'null'))

await n.navigate('https://github.com/deepseek-ai/deepseek-harness')
await sleep(7000)
await dshEval(`document.querySelector('button[aria-label="Pick element"]').click()`)
// Armed flag lives in the BROWSER page: poll it through the control plane.
let armed = false
for (let i = 0; i < 40 && !armed; i++) {
  await sleep(500)
  armed = (await plane('/eval', { js: '!!(window.__dshPick && window.__dshPick.active)' })
    .then((r) => r.result).catch(() => 'false')) === 'true'
}
console.log('browser armed:', armed)
await n.cdp('Input.dispatchMouseEvent', { type: 'mousePressed', x: 300, y: 300, button: 'left', clickCount: 1 })
await n.cdp('Input.dispatchMouseEvent', { type: 'mouseReleased', x: 300, y: 300, button: 'left', clickCount: 1 })
await sleep(4000)
const readout = await dshEval(`(() => {
  const spans = Array.from(document.querySelectorAll('span'));
  const line = spans.map((e) => e.textContent).find((s) => s && s.indexOf('selector') >= 0);
  const all = spans.map((e) => (e.textContent || '').slice(0, 60)).filter((s) => /pre|button|link|div|article/i.test(s));
  return JSON.stringify({ line: (line || '').slice(0, 200), candidates: all.slice(0, 8) });
})()`)
console.log('readout scan:', readout)
console.log('RACE-FREE DONE')
