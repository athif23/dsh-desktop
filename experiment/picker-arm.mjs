/** Arm the picker (90s) and print the ElementReference the human clicks. */
import { NativeSurface } from '../../dsh-browser/src/native.js'
import { pickElement } from '../../dsh-browser/src/picker.js'

const n = new NativeSurface('http://127.0.0.1:45331')
console.log('PICKER ARMED — hover the browser (magenta outline), click an element, or Esc to cancel')
const ref = await pickElement(n, { timeoutMs: 90000 })
console.log('PICKED: ' + JSON.stringify(ref, null, 1))

// Optional element shot when the box is sane.
const b = ref.box
if (b && b.width > 4 && b.height > 4 && b.width < 2000 && b.height < 2000) {
  try {
    const shot = await n.cdp('Page.captureScreenshot', {
      format: 'jpeg', quality: 70, fromSurface: true,
      clip: { x: Math.max(0, b.x), y: Math.max(0, b.y), width: b.width, height: b.height, scale: 1 },
    })
    const { writeFileSync } = await import('node:fs')
    writeFileSync(new URL('./pick-shot.jpg', import.meta.url), Buffer.from(shot.data, 'base64'))
    console.log('element shot saved (b64 len ' + shot.data.length + ')')
  } catch (e) {
    console.log('element shot skipped:', String(e.message ?? e).slice(0, 120))
  }
}
console.log('PICKER DONE')
