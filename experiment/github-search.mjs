import { NativeSurface } from '../../dsh-browser/src/native.js'
import { elementCenter, resolveRef, snapshotToRefs } from '../../dsh-browser/src/snapshot.js'

const n = new NativeSurface('http://127.0.0.1:45331')
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
let gen = 0

async function snap(label) {
  const [nodes, state] = await Promise.all([n.axTree(), n.getState()])
  gen += 1
  const s = snapshotToRefs(nodes, gen, state.url)
  console.log(`[${label}] ${state.url} refs=${s.refs.size}`)
  return s
}

async function clickRef(s, ref) {
  const el = await resolveRef(n, s, ref)
  const c = await elementCenter(n, el.objectId)
  console.log(`click ${ref} "${el.name}" at ${c.x},${c.y}`)
  await n.cdp('Input.dispatchMouseEvent', { type: 'mousePressed', x: c.x, y: c.y, button: 'left', clickCount: 1 })
  await n.cdp('Input.dispatchMouseEvent', { type: 'mouseReleased', x: c.x, y: c.y, button: 'left', clickCount: 1 })
}

await n.navigate('https://github.com/')
await sleep(5000)
const s1 = await snap('github-home')
const btn = [...s1.refs.entries()].find(([, v]) => v.role === 'button' && /search/i.test(v.name))?.[0]
console.log('search button:', btn)
await clickRef(s1, btn)
await sleep(3000)
const s2 = await snap('search-open')
const box = [...s2.refs.entries()].find(([, v]) => (v.role === 'textbox' || v.role === 'searchbox' || v.role === 'combobox'))?.[0]
console.log('search box:', box, JSON.stringify(s2.refs.get(box)))
const el = await resolveRef(n, s2, box)
await n.cdp('Runtime.callFunctionOn', { objectId: el.objectId, functionDeclaration: 'function() { this.focus(); }' })
await n.cdp('Input.insertText', { text: 'deepseek harness' })
await sleep(2000)
await n.cdp('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Enter', code: 'Enter' })
await n.cdp('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Enter', code: 'Enter' })
await sleep(5000)
console.log('after submit:', JSON.stringify(await n.getState()))
const s3 = await snap('search-results')
console.log(s3.text.split('\n').slice(0, 14).join('\n'))
console.log('GITHUB SEARCH DONE')
