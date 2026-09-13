/** Effect verification: submit text, scroll delta, screenshot dims. */
import { apply } from '../../dsh-browser/index.js'
import { writeFileSync } from 'node:fs'

const seen = new Map()
const fakeCtx = {
  tools: { register: (def) => { seen.set(def.name, def); return () => {} } },
  effect: (fn) => fn(),
}
apply(fakeCtx, { edgePath: process.execPath })
const tool = (name) => seen.get(name)
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const plane = (path, body) => fetch(`http://127.0.0.1:45331${path}`, {
  method: body === undefined ? 'GET' : 'POST',
  headers: { 'content-type': 'application/json' },
  body: body === undefined ? undefined : JSON.stringify(body),
}).then((r) => r.json())
const pageEval = (js) => plane('/eval', { js }).then((r) => JSON.parse(r.result ?? 'null'))

await tool('browser_navigate').execute({ url: new URL('./form.html', import.meta.url).href })
await sleep(2500)
await tool('browser_snapshot').execute({})
await tool('browser_type').execute({ target: '@e2', text: 'visible path', submit: true })
await sleep(1500)
console.log('out after tool type+submit:', await pageEval('JSON.stringify(document.getElementById("out").textContent)'))

await tool('browser_navigate').execute({ url: 'https://github.com/deepseek-ai/deepseek-harness' })
await sleep(5000)
const y0 = await pageEval('JSON.stringify(window.scrollY)')
await tool('browser_scroll').execute({ deltaY: 800 })
await sleep(1500)
const y1 = await pageEval('JSON.stringify(window.scrollY)')
console.log(`scrollY ${y0} → ${y1}`)

const shot = await tool('browser_screenshot').execute({})
writeFileSync(new URL('./visible-shot.jpg', import.meta.url), Buffer.from(shot.imageBase64, 'base64'))
console.log('saved visible-shot.jpg, b64 len:', shot.imageBase64.length)
console.log('VERIFY DONE')
