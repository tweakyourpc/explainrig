// Explainrig page engine. The page plays itself from load: every cue in CUES fires at its time (seconds
// from load, which is when Showrig starts recording the page). Cue times come from the narration timing.
const q = s => document.querySelector(s), sleep = ms => new Promise(r => setTimeout(r, ms));
const cur = q('#cursor');
let T0 = 0;
const at = async s => { const w = T0 + s * 1000 - performance.now(); if (w > 0) await sleep(w); };
async function move(el, ms = 700) {
  const r = el.getBoundingClientRect();
  const x = r.left + r.width / 2, y = r.top + r.height / 2;
  cur.style.opacity = 1;
  cur.style.transition = `transform ${ms}ms cubic-bezier(.45,0,.2,1),opacity .3s`;
  cur.style.transform = `translate(${x}px,${y}px)`;
  cur.dataset.x = x; cur.dataset.y = y;
  await sleep(ms + 80);
}
async function click(el) {
  await move(el);
  const rip = document.createElement('div');
  rip.className = 'ripple'; rip.style.left = cur.dataset.x + 'px'; rip.style.top = cur.dataset.y + 'px';
  document.body.appendChild(rip); setTimeout(() => rip.remove(), 600);
}
const ACTIONS = {
  show: el => el.classList.remove('hidden'),
  on: el => el.classList.add('on'),
  off: el => el.classList.remove('on'),
  width: (el, v) => { el.style.width = v; },
  click: el => click(el),
  hideCursor: () => { cur.style.opacity = 0; },
};
async function run() {
  const cues = [...CUES].sort((a, b) => a[0] - b[0]);
  for (const [t, action, sel, value] of cues) {
    await at(t);
    const el = sel ? q(sel) : null;
    if (sel && !el) continue;
    const f = ACTIONS[action];
    if (f) { const p = f(el, value); if (action === 'click') await p; }
  }
}
window.addEventListener('load', async () => {
  try { await document.fonts.ready; } catch (e) {}
  await Promise.all([...document.images].map(i => i.decode().catch(() => 0)));
  T0 = performance.now(); await run(); window.DONE = performance.now() / 1000;
});
