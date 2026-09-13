import { NativeSurface } from '../../dsh-browser/src/native.js'

const n = new NativeSurface('http://127.0.0.1:45331')
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const pe = (js) => n.eval(js).then((r) => JSON.parse(r ?? 'null'))

await n.navigate(new URL('./form.html', import.meta.url).href)
await sleep(2500)
// Focus + insert via the same calls the tool uses, then inspect.
await n.cdp('Runtime.evaluate', { expression: 'document.getElementById("q").focus()' })
await n.cdp('Input.insertText', { text: 'dbg' })
console.log('active:', await pe('JSON.stringify(document.activeElement && document.activeElement.id)'))
console.log('value:', await pe('JSON.stringify(document.getElementById("q").value)'))

// Variant A: rawKeyDown/rawKeyUp with vk.
await n.cdp('Input.dispatchKeyEvent', { type: 'rawKeyDown', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13, nativeVirtualKeyCode: 13 })
await n.cdp('Input.dispatchKeyEvent', { type: 'rawKeyUp', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13, nativeVirtualKeyCode: 13 })
await sleep(1000)
console.log('out after rawKeyDown Enter:', await pe('JSON.stringify(document.getElementById("out").textContent)'))
console.log('KEY DEBUG DONE')
