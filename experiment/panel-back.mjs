/** Panel Back button drives the VISIBLE (native) browser, not the stream. */
import { NativeSurface } from '../../dsh-browser/src/native.js'

const n = new NativeSurface('http://127.0.0.1:45331')
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const plane = (path, body) => fetch(`http://127.0.0.1:45331${path}`, {
  method: body === undefined ? 'GET' : 'POST',
  headers: { 'content-type': 'application/json' },
  body: body === undefined ? undefined : JSON.stringify(body),
}).then((r) => r.json())
const dshEval = (js) => plane('/dsh-eval', { js }).then((r) => JSON.parse(r.result ?? 'null'))

await n.navigate('https://example.com/')
await sleep(3000)
await n.navigate('https://github.com/deepseek-ai/deepseek-harness')
await sleep(5000)
console.log('before panel-back:', (await n.getState()).url)
await dshEval(`document.querySelector('button[aria-label="Back"]').click()`)
await sleep(4000)
console.log('after panel-back:', (await n.getState()).url)
console.log('PANEL BACK DONE')
