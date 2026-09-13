/** Capture the exact /input/pick response body the panel receives. */
import { NativeSurface } from '../../dsh-browser/src/native.js'

const n = new NativeSurface('http://127.0.0.1:45331')
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

await n.navigate('https://github.com/deepseek-ai/deepseek-harness')
await sleep(7000)
// Fire the route exactly like the panel does; click mid-hold.
const pending = fetch('http://127.0.0.1:9333/input/pick', {
  method: 'POST', headers: { 'content-type': 'application/json' },
  body: JSON.stringify({ timeoutMs: 30000 }),
}).then(async (res) => ({ status: res.status, body: (await res.text()).slice(0, 600) }))
await sleep(2500)
await n.cdp('Input.dispatchMouseEvent', { type: 'mousePressed', x: 350, y: 250, button: 'left', clickCount: 1 })
await n.cdp('Input.dispatchMouseEvent', { type: 'mouseReleased', x: 350, y: 250, button: 'left', clickCount: 1 })
console.log('route response:', JSON.stringify(await pending).slice(0, 700))
console.log('ROUTE BODY DONE')
