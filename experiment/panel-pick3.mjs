/** Tight single-script panel pick: press, confirm picking state, click, read. */
import { NativeSurface } from '../../dsh-browser/src/native.js'

const n = new NativeSurface('http://127.0.0.1:45331')
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const plane = (path, body) => fetch(`http://127.0.0.1:45331${path}`, {
  method: body === undefined ? 'GET' : 'POST',
  headers: { 'content-type': 'application/json' },
  body: body === undefined ? undefined : JSON.stringify(body),
}).then((r) => r.json())
const dshEval = (js) => plane('/dsh-eval', { js }).then((r) => JSON.parse(r.result ?? 'null'))
const pickDis = () => dshEval(`document.querySelector('button[aria-label="Pick element"]').disabled`)

await n.navigate('https://github.com/deepseek-ai/deepseek-harness')
await sleep(7000)
await dshEval(`document.querySelector('button[aria-label="Pick element"]').click()`)
await sleep(1200)
console.log('picking right after press (want true):', await pickDis())
let armed = false
for (let i = 0; i < 30 && !armed; i++) {
  await sleep(500)
  armed = (await plane('/eval', { js: '!!(window.__dshPick && window.__dshPick.active)' })
    .then((r) => r.result).catch(() => 'false')) === 'true'
}
console.log('browser armed:', armed)
await n.cdp('Input.dispatchMouseEvent', { type: 'mousePressed', x: 350, y: 250, button: 'left', clickCount: 1 })
await n.cdp('Input.dispatchMouseEvent', { type: 'mouseReleased', x: 350, y: 250, button: 'left', clickCount: 1 })
await sleep(3000)
console.log('picking after click (want false):', await pickDis())
const readout = await dshEval(`(() => {
  const spans = Array.from(document.querySelectorAll('span'));
  const hits = spans.map((e) => e.textContent).filter((s) => s && s.indexOf(' \\u00b7 ') >= 0);
  return JSON.stringify(hits.map((s) => s.slice(0, 160)).slice(0, 4));
})()`)
console.log('readout:', readout)
console.log('TIGHT DONE')
