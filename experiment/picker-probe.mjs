import { NativeSurface } from '../../dsh-browser/src/native.js'

const n = new NativeSurface('http://127.0.0.1:45331')
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const pe = (js) => n.eval(js).then((r) => JSON.parse(r ?? 'null'))

await n.navigate(new URL('./form.html', import.meta.url).href)
await sleep(2500)
console.log('ready:', await pe('JSON.stringify(document.readyState)'))
console.log('viewport:', await pe('JSON.stringify({w: innerWidth, h: innerHeight})'))
console.log('button rect:', await pe('JSON.stringify((() => { const r = document.getElementById("go").getBoundingClientRect(); return {x: r.x, y: r.y, w: r.width, h: r.height}; })())'))
console.log('at point:', await pe('JSON.stringify((() => { const el = document.elementFromPoint(254, 91); return el ? el.outerHTML.slice(0,80) : null; })())'))
// Arm manually and watch hover state via a marker.
await n.eval('window.__dbgHover = null')
await n.eval(`(function() {
  window.__dshDbg = [];
  addEventListener('mousemove', (e) => { window.__dshDbg.push([e.clientX, e.clientY]); }, true);
})()`)
await n.cdp('Input.dispatchMouseEvent', { type: 'mouseMoved', x: 254, y: 91 })
await sleep(800)
console.log('mousemove seen by page:', await pe('JSON.stringify(window.__dshDbg)'))
console.log('PICKER PROBE DONE')
