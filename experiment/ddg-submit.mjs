import { NativeSurface } from '../../dsh-browser/src/native.js'
import { resolveRef, snapshotToRefs } from '../../dsh-browser/src/snapshot.js'

const n = new NativeSurface('http://127.0.0.1:45331')
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

await n.navigate('https://html.duckduckgo.com/html/')
await sleep(4000)
const state = await n.getState()
const snap = snapshotToRefs(await n.axTree(), 1, state.url)
console.log(snap.text.split('\n').slice(0, 12).join('\n'))
const ref = [...snap.refs.entries()].find(([, v]) => v.role === 'textbox')?.[0]
console.log('box:', ref)
const el = await resolveRef(n, snap, ref)
await n.cdp('Runtime.callFunctionOn', { objectId: el.objectId, functionDeclaration: 'function() { this.focus(); this.value = ""; }' })
await n.cdp('Input.insertText', { text: 'deepseek harness' })
await n.cdp('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Enter', code: 'Enter' })
await n.cdp('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Enter', code: 'Enter' })
await sleep(5000)
console.log('after submit:', JSON.stringify(await n.getState()))
const snap2 = snapshotToRefs(await n.axTree(), 2, (await n.getState()).url)
console.log(snap2.text.split('\n').slice(0, 10).join('\n'))
console.log('DDG SUBMIT DONE')
