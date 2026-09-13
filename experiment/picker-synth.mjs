import { NativeSurface } from '../../dsh-browser/src/native.js'
import { pickElement } from '../../dsh-browser/src/picker.js'

const n = new NativeSurface('http://127.0.0.1:45331')
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const pe = (js) => n.eval(js).then((r) => JSON.parse(r ?? 'null'))

await n.navigate(new URL('./form.html', import.meta.url).href)
await sleep(2500)
console.log('overlay visible:', await n.isOverlayVisible())

// Arm WITHOUT blocking: start pick, drive synthetic events, await result.
const picked = pickElement(n, { timeoutMs: 30000 })
await sleep(1000)
console.log('pick state after arm:', await pe('JSON.stringify(window.__dshPick ? {active: window.__dshPick.active} : null)'))
// Synthetic hover + click where the Search button is (~254,91 per earlier run).
await n.cdp('Input.dispatchMouseEvent', { type: 'mouseMoved', x: 254, y: 91 })
await sleep(500)
await n.cdp('Input.dispatchMouseEvent', { type: 'mousePressed', x: 254, y: 91, button: 'left', clickCount: 1 })
await n.cdp('Input.dispatchMouseEvent', { type: 'mouseReleased', x: 254, y: 91, button: 'left', clickCount: 1 })
try {
  const ref = await picked
  console.log('PICKED:', JSON.stringify(ref).slice(0, 400))
} catch (e) {
  console.log('pick failed:', String(e.message).slice(0, 160))
}
console.log('PICKER SYNTH DONE')
