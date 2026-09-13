/**
 * Phase-1 live loop: drive the EXACT native WebView2 from Node using the
 * same modules the agent tools use (src/native.js + src/snapshot.js).
 * Run: node experiment/live-loop.mjs   (dsh-desktop shell must be up)
 */
import { NativeSurface } from '../../dsh-browser/src/native.js'
import { elementCenter, resolveRef, snapshotToRefs } from '../../dsh-browser/src/snapshot.js'

const native = new NativeSurface('http://127.0.0.1:45331')
let gen = 0
const step = (n) => console.log(`\n===== ${n} =====`)

async function snapshot(label) {
  const [nodes, state] = await Promise.all([native.axTree(), native.getState()])
  gen += 1
  const snap = snapshotToRefs(nodes, gen, state.url)
  console.log(`[${label}] url=${state.url} refs=${snap.refs.size}`)
  console.log(snap.text.split('\n').slice(0, 25).join('\n'))
  return { snap, state }
}

async function clickRef(snap, ref) {
  const el = await resolveRef(native, snap, ref)
  const center = await elementCenter(native, el.objectId)
  if (center === undefined) throw new Error(`no box for ${ref}`)
  console.log(`click ${ref} (${el.role} "${el.name}") at ${center.x},${center.y}`)
  await native.cdp('Input.dispatchMouseEvent', { type: 'mousePressed', x: center.x, y: center.y, button: 'left', clickCount: 1 })
  await native.cdp('Input.dispatchMouseEvent', { type: 'mouseReleased', x: center.x, y: center.y, button: 'left', clickCount: 1 })
}

const sleep = (ms) => new Promise(r => setTimeout(r, ms))

// 1. GitHub loop: snapshot → click a link by ref only → verify state changed.
step('state')
console.log(JSON.stringify(await native.getState()))
step('navigate github repo')
await native.navigate('https://github.com/deepseek-ai/deepseek-harness')
await sleep(5000)
let { snap } = await snapshot('repo')
const linkRef = [...snap.refs.entries()].find(([, v]) => v.role === 'link' && /insights|discussions|security and quality/i.test(v.name))?.[0]
  ?? [...snap.refs.keys()].find(k => /insights/i.test(snap.refs.get(k).name))
console.log('chose link ref:', linkRef, JSON.stringify(snap.refs.get(linkRef)))
await clickRef(snap, linkRef)
await sleep(5000)
let after = await snapshot('after-click')
console.log('url moved:', after.state.url)

// 2. Search on Google: textbox ref → type + submit → results.
step('search via textbox ref')
await native.navigate('https://www.google.com/')
await sleep(5000)
after = await snapshot('google')
const boxRef = [...after.snap.refs.entries()].find(([, v]) => (v.role === 'textbox' || v.role === 'searchbox' || v.role === 'combobox'))?.[0]
console.log('chose box ref:', boxRef, JSON.stringify(after.snap.refs.get(boxRef)))
if (boxRef !== undefined) {
  const el = await resolveRef(native, after.snap, boxRef)
  await native.cdp('Runtime.callFunctionOn', { objectId: el.objectId, functionDeclaration: 'function() { this.focus(); }' })
  await native.cdp('Input.insertText', { text: 'deepseek harness' })
  await native.cdp('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Enter', code: 'Enter' })
  await native.cdp('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Enter', code: 'Enter' })
  await sleep(5000)
  await snapshot('after-search')
} else {
  console.log('no textbox ref: consent wall or layout changed; inspect snapshot above')
}

// 3. Scroll: needs the visible overlay (hidden compositor stalls wheel).
step('agent scroll')
try {
  await native.requireVisible('scroll')
  const y0 = JSON.parse(await native.eval('JSON.stringify(window.scrollY)'))
  await native.cdp('Input.dispatchMouseEvent', { type: 'mouseWheel', x: 400, y: 400, deltaX: 0, deltaY: 800 })
  await sleep(1500)
  const y1 = JSON.parse(await native.eval('JSON.stringify(window.scrollY)'))
  console.log(`scrollY ${y0} → ${y1} (delta ${y1 - y0})`)
} catch (e) {
  console.log('scroll skipped:', String(e.message).slice(0, 130))
}

// 4. Stale: snapshot, navigate away, use old ref → must refuse.
step('stale ref behavior')
const staleSnap = (await snapshot('stale-source')).snap
await native.navigate('https://example.com/')
await sleep(3000)
try {
  await resolveRef(native, staleSnap, [...staleSnap.refs.keys()][0] ?? '@e1')
  console.log('STALE-CHECK FAILED: old ref resolved after navigation')
} catch (e) {
  console.log('stale refused as designed:', String(e.message).slice(0, 140))
}

// 5. Iframe page: what does the AX snapshot see?
step('iframe page')
await native.navigate('https://www.w3schools.com/html/html_iframe.asp')
await sleep(5000)
const frame = await snapshot('iframe-page')
console.log(`refs on iframe page: ${frame.snap.refs.size}`)

// 6. Drag-select observability (needs visible overlay like scroll).
step('selection readout')
await native.navigate('https://example.com/')
await sleep(3000)
try {
  await native.requireVisible('selection')
  await native.cdp('Input.dispatchMouseEvent', { type: 'mousePressed', x: 200, y: 120, button: 'left', clickCount: 1 })
  await native.cdp('Input.dispatchMouseEvent', { type: 'mouseMoved', x: 500, y: 160, button: 'left' })
  await native.cdp('Input.dispatchMouseEvent', { type: 'mouseReleased', x: 500, y: 160, button: 'left', clickCount: 1 })
  const sel = await native.eval('JSON.stringify(window.getSelection ? window.getSelection().toString().slice(0,120) : "")')
  console.log('selected text:', sel)
} catch (e) {
  console.log('selection skipped:', String(e.message).slice(0, 130))
}

console.log('\nLIVE LOOP DONE')
