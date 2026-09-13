/** Panel pick, machine-completed: press ⌖, CDP-click, read readout. */
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
// Press panel Pick (don't await the long-poll here; panel holds it).
const pressP = plane('/dsh-eval', {
  js: `(document.querySelector('button[aria-label="Pick element"]').click(), 'pressed')`,
})
await sleep(1500)
// Click a real element via CDP while armed.
await n.cdp('Input.dispatchMouseEvent', { type: 'mousePressed', x: 300, y: 300, button: 'left', clickCount: 1 })
await n.cdp('Input.dispatchMouseEvent', { type: 'mouseReleased', x: 300, y: 300, button: 'left', clickCount: 1 })
await pressP.catch(() => ({}))
await sleep(4000)
const readout = await dshEval(`(() => {
  const spans = Array.from(document.querySelectorAll('span'));
  const line = spans.map((e) => e.textContent).find((s) => s && s.includes(' \\u00b7 ') && (s.includes('#') || s.includes(' > ')));
  return JSON.stringify((line || '').slice(0, 220));
})()`)
console.log('panel pick readout:', readout)
console.log('PANEL PICK DONE')
