import { NativeSurface } from '../../dsh-browser/src/native.js'
import { writeFileSync } from 'node:fs'

const n = new NativeSurface('http://127.0.0.1:45331')
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const pe = (js) => n.eval(js).then((r) => JSON.parse(r ?? 'null'))

await n.navigate('https://github.com/deepseek-ai/deepseek-harness')
await sleep(6000)
// Fresh box: scroll the citation <pre> into view, then measure.
const box = await pe(`JSON.stringify((() => {
  const els = Array.from(document.querySelectorAll('pre'));
  const el = els[4] || els[0];
  if (!el) return null;
  el.scrollIntoView({ block: 'center' });
  const r = el.getBoundingClientRect();
  return { x: Math.round(r.x), y: Math.round(r.y), width: Math.round(r.width), height: Math.round(r.height) };
})())`)
console.log('fresh box:', JSON.stringify(box))
await sleep(800)
const shot = await n.cdp('Page.captureScreenshot', {
  format: 'jpeg', quality: 70, fromSurface: true, captureBeyondViewport: true,
  clip: { x: box.x, y: box.y, width: Math.min(box.width, 800), height: Math.min(box.height, 600), scale: 1 },
})
writeFileSync(new URL('./pick-shot.jpg', import.meta.url), Buffer.from(shot.data, 'base64'))
console.log('element shot saved, b64 len:', shot.data.length)
console.log('CLIP RETRY DONE')
