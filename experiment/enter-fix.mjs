/** Confirm the Enter fix at the TOOL layer (type + submit:true). */
import { apply } from '../../dsh-browser/index.js'

const seen = new Map()
const fakeCtx = {
  tools: { register: (def) => { seen.set(def.name, def); return () => {} } },
  effect: (fn) => fn(),
}
apply(fakeCtx, { edgePath: process.execPath })
const tool = (name) => seen.get(name)
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const out = () => fetch('http://127.0.0.1:45331/eval', {
  method: 'POST', headers: { 'content-type': 'application/json' },
  body: JSON.stringify({ js: 'JSON.stringify(document.getElementById("out").textContent)' }),
}).then((r) => r.json()).then((r) => JSON.parse(r.result))

await tool('browser_navigate').execute({ url: new URL('./form.html', import.meta.url).href })
await sleep(2500)
await tool('browser_snapshot').execute({})
console.log('type+submit:', JSON.stringify(await tool('browser_type').execute({ target: '@e2', text: 'tool enter', submit: true })))
await sleep(1500)
console.log('out:', await out())
console.log('ENTER-FIX DONE')
