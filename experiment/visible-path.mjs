/** Visible-path validation through the SHIPPED tools (overlay must be up). */
import { apply } from '../../dsh-browser/index.js'

const seen = new Map()
const fakeCtx = {
  tools: { register: (def) => { seen.set(def.name, def); return () => {} } },
  effect: (fn) => fn(),
}
apply(fakeCtx, { edgePath: process.execPath })
const tool = (name) => seen.get(name)
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

// Enter-submit fix: type + submit:true must submit the form now.
await tool('browser_navigate').execute({ url: new URL('./form.html', import.meta.url).href })
await sleep(2500)
await tool('browser_snapshot').execute({})
console.log('type+submit:', JSON.stringify(await tool('browser_type').execute({ target: '@e2', text: 'visible path', submit: true })))
await sleep(1500)

// Button click through the tool layer.
await tool('browser_navigate').execute({ url: new URL('./form.html', import.meta.url).href })
await sleep(2500)
await tool('browser_snapshot').execute({})
console.log('click:', JSON.stringify(await tool('browser_click').execute({ target: '@e3' })))
await sleep(1000)

// Scroll on a tall page: scrollY must move.
await tool('browser_navigate').execute({ url: 'https://github.com/deepseek-ai/deepseek-harness' })
await sleep(5000)
const st0 = await tool('browser_get_state').execute({})
console.log('viewport:', JSON.stringify(st0.viewport))
console.log('scroll:', JSON.stringify(await tool('browser_scroll').execute({ deltaY: 800 })))
await sleep(1500)
const st1 = await tool('browser_get_state').execute({})
console.log('after scroll, back-avail:', st1.canGoBack)

// Back/forward through tools.
console.log('back:', JSON.stringify(await tool('browser_back').execute({})))
await sleep(3000)
console.log('after back:', (await tool('browser_get_state').execute({})).url)
console.log('forward:', JSON.stringify(await tool('browser_forward').execute({})))
await sleep(3000)
console.log('after forward:', (await tool('browser_get_state').execute({})).url)

// Screenshot: must come from the visible page with matching dims.
const shot = await tool('browser_screenshot').execute({})
const bytes = Math.round(shot.imageBase64.length / 4 * 3)
console.log(`screenshot bytes=${bytes} viewport=${st0.viewport.w}x${st0.viewport.h}`)
console.log('VISIBLE-PATH DONE')
