/** Resize tracking: dims follow the new panel size, clicks stay correct. */
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

const st = await tool('browser_get_state').execute({})
console.log('new viewport:', JSON.stringify(st.viewport), 'url:', st.url)
const shot = await tool('browser_screenshot').execute({})
writeFileSync(new URL('./resize-shot.jpg', import.meta.url), Buffer.from(shot.imageBase64, 'base64'))
console.log('shot b64 len:', shot.imageBase64.length)

// Fresh snapshot on the new geometry → click first real link → verify nav.
await tool('browser_navigate').execute({ url: 'https://github.com/deepseek-ai/deepseek-harness' })
await sleep(5000)
const snap = await tool('browser_snapshot').execute({})
const lines = snap.text.split('\n')
const discussions = lines.find((l) => /discussions/i.test(l))
console.log('post-resize ref line:', discussions)
const ref = discussions.split(' ')[0]
console.log('click:', JSON.stringify(await tool('browser_click').execute({ target: ref })))
await sleep(4000)
console.log('after click:', (await tool('browser_get_state').execute({})).url)
console.log('RESIZE DONE')
