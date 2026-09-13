/** Remaining proofs: youtube presence, drag-select readout, element clip shot. */
import { NativeSurface } from '../../dsh-browser/src/native.js'
import { writeFileSync } from 'node:fs'

const n = new NativeSurface('http://127.0.0.1:45331')
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const pe = (js) => n.eval(js).then((r) => JSON.parse(r ?? 'null'))

// YouTube: loads + video element present (playback feel is human judgment).
await n.navigate('https://www.youtube.com/')
await sleep(6000)
console.log('yt title:', await pe('JSON.stringify(document.title).slice(0,80)'))
console.log('yt video count:', await pe('JSON.stringify(document.querySelectorAll("video").length)'))
console.log('yt state:', JSON.stringify(await n.getState()))

// Drag-select on the repo README, read back (overlay is up).
await n.navigate('https://github.com/deepseek-ai/deepseek-harness')
await sleep(5000)
await n.cdp('Input.dispatchMouseEvent', { type: 'mousePressed', x: 200, y: 200, button: 'left', clickCount: 1 })
await n.cdp('Input.dispatchMouseEvent', { type: 'mouseMoved', x: 500, y: 260, button: 'left' })
await n.cdp('Input.dispatchMouseEvent', { type: 'mouseReleased', x: 500, y: 260, button: 'left', clickCount: 1 })
console.log('selection:', await pe('JSON.stringify(window.getSelection().toString().slice(0,120))'))

// Element clip shot of the picked <pre> box (x49 y380 510x185, scale 1).
const shot = await n.cdp('Page.captureScreenshot', {
  format: 'jpeg', quality: 70, fromSurface: true,
  clip: { x: 49, y: 380, width: 510, height: 185, scale: 1 },
})
writeFileSync(new URL('./pick-shot.jpg', import.meta.url), Buffer.from(shot.data, 'base64'))
console.log('element shot saved, b64 len:', shot.data.length)
console.log('REMAINING DONE')
