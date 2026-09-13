import { NativeSurface } from '../../dsh-browser/src/native.js'
import { snapshotToRefs } from '../../dsh-browser/src/snapshot.js'

const n = new NativeSurface('http://127.0.0.1:45331')
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
await n.navigate('https://github.com/')
await sleep(5000)
const [nodes, state] = await Promise.all([n.axTree(), n.getState()])
const s = snapshotToRefs(nodes, 1, state.url)
console.log(s.text)
console.log('PROBE DONE')
