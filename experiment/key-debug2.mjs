import { NativeSurface } from '../../dsh-browser/src/native.js'

const n = new NativeSurface('http://127.0.0.1:45331')
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const pe = (js) => n.eval(js).then((r) => JSON.parse(r ?? 'null'))

await n.navigate(new URL('./form.html', import.meta.url).href)
await sleep(2500)
await n.cdp('Runtime.evaluate', { expression: 'document.getElementById("q").focus()' })
await n.cdp('Input.insertText', { text: 'with-cr' })
// keyDown Enter WITH text '\r' (char-producing Enter).
await n.cdp('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Enter', code: 'Enter', text: '\r', windowsVirtualKeyCode: 13, nativeVirtualKeyCode: 13 })
await n.cdp('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13, nativeVirtualKeyCode: 13 })
await sleep(1000)
console.log('out after Enter+text:', await pe('JSON.stringify(document.getElementById("out").textContent)'))
console.log('ENTER VARIANT DONE')
