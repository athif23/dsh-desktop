const plane = (path, body) => fetch(`http://127.0.0.1:45331${path}`, {
  method: body === undefined ? 'GET' : 'POST',
  headers: { 'content-type': 'application/json' },
  body: body === undefined ? undefined : JSON.stringify(body),
}).then((r) => r.json())

// Press the panel's Pick button the way a human would (real wiring:
// button -> POST /input/pick long-poll -> native arm).
const clicked = await plane('/dsh-eval', {
  js: `(() => {
    const b = document.querySelector('button[aria-label="Pick element"]');
    if (!b) return JSON.stringify({ clicked: false });
    b.click();
    return JSON.stringify({ clicked: true, disabled: b.disabled });
  })()`,
}).then((r) => JSON.parse(r.result ?? '{}'))
console.log('pick button:', JSON.stringify(clicked))
console.log('PRESS DONE')
