/** Human picker, second attempt (settle fix): repo page, settled, 90s. */
import { NativeSurface } from '../../dsh-browser/src/native.js'
import { pickElement } from '../../dsh-browser/src/picker.js'

const n = new NativeSurface('http://127.0.0.1:45331')
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

await n.navigate('https://github.com/deepseek-ai/deepseek-harness')
await sleep(8000)
console.log('PICKER ARMED on settled repo page — hover (magenta outline) + click, or Esc')
const ref = await pickElement(n, { timeoutMs: 90000 })
console.log('PICKED: ' + JSON.stringify(ref))
console.log('PICKER HUMAN DONE')
