import { NativeSurface } from '../../dsh-browser/src/native.js'
import { elementCenter, resolveRef, snapshotToRefs } from '../../dsh-browser/src/snapshot.js'

const n = new NativeSurface('http://127.0.0.1:45331')
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

await n.navigate(new URL('./form.html', import.meta.url).href)
await sleep(3000)
const [nodes, state] = await Promise.all([n.axTree(), n.getState()])
const s = snapshotToRefs(nodes, 1, state.url)
console.log(s.text)
const box = [...s.refs.entries()].find(([, v]) => v.role === 'textbox')?.[0]
const btn = [...s.refs.entries()].find(([, v]) => v.role === 'button' && /search/i.test(v.name))?.[0]
console.log('box:', box, 'button:', btn)

// Type via ref, submit via Enter.
const el = await resolveRef(n, s, box)
await n.cdp('Runtime.callFunctionOn', { objectId: el.objectId, functionDeclaration: 'function() { this.focus(); }' })
await n.cdp('Input.insertText', { text: 'deepseek harness' })
await n.cdp('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Enter', code: 'Enter' })
await n.cdp('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Enter', code: 'Enter' })
await sleep(1500)
console.log('out after Enter-submit:', await n.eval('JSON.stringify(document.getElementById("out").textContent)'))

// Reset, type again, click the submit button via ref instead.
await n.eval('document.getElementById("out").textContent = "not submitted"')
const el2 = await resolveRef(n, s, box)
await n.cdp('Runtime.callFunctionOn', { objectId: el2.objectId, functionDeclaration: 'function() { this.focus(); this.value = ""; }' })
await n.cdp('Input.insertText', { text: 'second try' })
const elb = await resolveRef(n, s, btn)
const c = await elementCenter(n, elb.objectId)
console.log(`click ${btn} at ${c.x},${c.y}`)
await n.cdp('Input.dispatchMouseEvent', { type: 'mousePressed', x: c.x, y: c.y, button: 'left', clickCount: 1 })
await n.cdp('Input.dispatchMouseEvent', { type: 'mouseReleased', x: c.x, y: c.y, button: 'left', clickCount: 1 })
await sleep(1500)
console.log('out after button-click:', await n.eval('JSON.stringify(document.getElementById("out").textContent)'))
console.log('FORM SUBMIT DONE')
