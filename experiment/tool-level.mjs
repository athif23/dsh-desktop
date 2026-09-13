/**
 * Tool-level live test: use the SHIPPED tool definitions (apply + fake ctx)
 * against the running shell — the exact path the model calls.
 */
import { apply } from '../../dsh-browser/index.js'

const seen = new Map()
const fakeCtx = {
  tools: { register: (def) => { seen.set(def.name, def); return () => {} } },
  effect: (fn) => fn(),
}
apply(fakeCtx, { edgePath: process.execPath })
const tool = (name) => seen.get(name)
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

console.log('tools:', [...seen.keys()].join(','))
console.log('get_state:', JSON.stringify(await tool('browser_get_state').execute({})))

// Form: snapshot → type @e2 + submit:true → Enter must submit now.
await tool('browser_navigate').execute({ url: new URL('./form.html', import.meta.url).href })
await sleep(2500)
const snap = await tool('browser_snapshot').execute({})
console.log(snap.text)
console.log('type+submit:', JSON.stringify(await tool('browser_type').execute({ target: '@e2', text: 'tool layer', submit: true })))
await sleep(1500)

// Button click via @e ref through the tool layer (lastSnapshot store path).
console.log('click:', JSON.stringify(await tool('browser_click').execute({ target: '@e3' })))
await sleep(1000)

// Stale: move page, reuse @e2 → must refuse.
await tool('browser_navigate').execute({ url: 'https://example.com/' })
await sleep(2500)
try {
  await tool('browser_click').execute({ target: '@e2' })
  console.log('STALE-CHECK FAILED')
} catch (e) {
  console.log('stale refused:', String(e.message).slice(0, 120))
}

// Hidden-tab guard: overlay is closed in this window → fast error, no hang.
for (const [name, args] of [['browser_scroll', { deltaY: 300 }], ['browser_screenshot', {}], ['browser_key', { key: 'Enter' }]]) {
  const t0 = Date.now()
  try {
    await tool(name).execute(args)
    console.log(name, 'UNEXPECTEDLY RAN (overlay visible?)')
  } catch (e) {
    console.log(name, `guarded in ${Date.now() - t0}ms:`, String(e.message).slice(0, 90))
  }
}
console.log('TOOL-LEVEL DONE')
