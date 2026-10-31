//! preview.html: the film as a timed slideshow in the browser. Pages play in an iframe, title scenes
//! are drawn as cards, captions follow the narration, and after the prepare pass the real voice
//! lines play at their times. No build step and no audio code in the app.

use explainrig_guess::Project;
use serde_json::json;

use crate::pages::esc;
use crate::timing::Timing;

pub fn html(p: &Project, t: &Timing) -> String {
    let scenes: Vec<serde_json::Value> = p
        .scenes
        .iter()
        .map(|s| {
            let st = t.scenes.get(&s.id).cloned().unwrap_or_default();
            let page = !crate::is_title(s);
            json!({
                "id": s.id,
                "title": s.title,
                "subtitle": s.subtitle,
                "start": st.film_start,
                "duration": st.film_duration,
                "page": if page { format!("pages/{}.html", s.id) } else { String::new() },
                "status": format!("{:?}", s.status).to_lowercase(),
            })
        })
        .collect();
    let data = json!({
        "title": p.title,
        "accent": p.style.accent,
        "background": p.style.background,
        "measured": t.measured,
        "duration": t.duration,
        "scenes": scenes,
        "narration": t.narration,
    });
    let data = serde_json::to_string(&data)
        .unwrap_or_else(|_| "{}".into())
        .replace("</", "<\\/");
    format!(
        r##"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>{title} · preview</title>
<style>
:root{{--bg:#0f1420;--panel:#171e2e;--ink:#e8ecf3;--mute:#94a0b8;--line:#2a3348;--accent:{accent}}}
*{{box-sizing:border-box;margin:0}} [hidden]{{display:none!important}}
body{{background:var(--bg);color:var(--ink);font:15px/1.5 Inter,system-ui,sans-serif;padding:20px 16px}}
main{{max-width:1000px;margin:0 auto;display:flex;flex-direction:column;gap:14px}}
h1{{font-size:20px}} .mute{{color:var(--mute)}}
.banner{{background:#3b2a0a;color:#ffd58a;border-radius:8px;padding:8px 12px;font-size:14px}}
.screen{{position:relative;width:100%;aspect-ratio:16/9;background:#000;border-radius:10px;overflow:hidden}}
.screen iframe{{position:absolute;left:0;top:0;width:1536px;height:864px;border:0;transform-origin:0 0}}
.card{{position:absolute;inset:0;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:12px;
text-align:center;padding:5%;background:radial-gradient(circle at 50% 30%,var(--accent) -60%,{bg} 70%)}}
.card b{{font-size:clamp(20px,4.6vw,46px);letter-spacing:-.5px}} .card span{{font-size:clamp(13px,2vw,20px);color:#d7deeb}}
.cap{{position:absolute;left:50%;bottom:5%;transform:translateX(-50%);max-width:86%;text-align:center;
background:#000c;color:#fff;padding:4px 10px;border-radius:4px;font-size:clamp(12px,1.8vw,18px)}}
.cap:empty{{display:none}}
.ctl{{display:flex;gap:12px;align-items:center}} .ctl input{{flex:1}}
button{{background:var(--accent);color:#1d1300;border:0;border-radius:8px;padding:8px 16px;font-weight:700;cursor:pointer}}
.strip{{display:flex;gap:3px;height:54px}}
.strip div{{background:var(--panel);border:1px solid var(--line);border-radius:6px;padding:4px 6px;overflow:hidden;cursor:pointer;
font-size:12px;white-space:nowrap;text-overflow:ellipsis;min-width:0}}
.strip div.on{{border-color:var(--accent);background:#2a2412}} .strip div i{{display:block;font-style:normal;color:var(--mute)}}
.strip div.approved{{box-shadow:inset 0 -3px 0 #2fa36b}}
</style></head><body><main>
<h1>{title} <span class="mute">· preview</span></h1>
<div class="banner" id="banner" hidden>Estimated timings and no voice yet. Run Prepare in Explainrig to hear the real narration with exact timings.</div>
<div class="screen" id="screen"><iframe id="frame" title="scene page"></iframe><div class="card" id="card"><b id="ct"></b><span id="cs"></span></div><div class="cap" id="cap"></div></div>
<div class="ctl"><button id="play">Play</button><span id="time" class="mute">0:00</span><input type="range" id="seek" min="0" step="0.1" value="0"><span id="total" class="mute"></span></div>
<div class="strip" id="strip"></div>
<p class="mute">Page animations restart from the beginning of their scene when you jump. Scenes approved in Explainrig have a green edge.</p>
</main>
<script>
const D = {data};
const $ = id => document.getElementById(id);
const fmt = s => `${{Math.floor(s / 60)}}:${{String(Math.floor(s % 60)).padStart(2, '0')}}`;
$('banner').hidden = D.measured; $('seek').max = D.duration; $('total').textContent = fmt(D.duration);
const strip = $('strip');
D.scenes.forEach((s, i) => {{
  const d = document.createElement('div');
  d.style.flex = Math.max(s.duration, 0.5); d.className = s.status === 'approved' ? 'approved' : '';
  d.innerHTML = `${{s.title.replace(/[<&]/g, '')}}<i>${{s.duration.toFixed(1)}} s</i>`;
  d.onclick = () => jump(s.start + 0.01); strip.appendChild(d);
}});
const audio = D.narration.map(n => n.audio ? new Audio(n.audio) : null);
let now = 0, playing = false, last = 0, current = -1, played = new Set();
function fit() {{ const w = $('screen').clientWidth; $('frame').style.transform = `scale(${{w / 1536}})`; }}
addEventListener('resize', fit); fit();
function sceneAt(t) {{ let k = 0; D.scenes.forEach((s, i) => {{ if (t >= s.start) k = i; }}); return k; }}
function show(i) {{
  current = i; const s = D.scenes[i];
  [...strip.children].forEach((d, k) => d.classList.toggle('on', k === i));
  if (s.page) {{ $('card').hidden = true; $('frame').hidden = false; $('frame').src = s.page + '?t=' + Date.now(); }}
  else {{ $('frame').hidden = true; $('frame').src = 'about:blank'; $('card').hidden = false; $('ct').textContent = s.title; $('cs').textContent = s.subtitle; }}
}}
function stopAll() {{ audio.forEach(a => a && a.pause()); }}
function jump(t) {{ now = t; stopAll(); played = new Set(D.narration.map((n, i) => n.start < t ? i : -1).filter(i => i >= 0)); show(sceneAt(t)); tick(true); }}
function tick(once) {{
  const ts = performance.now();
  if (playing) now += (ts - last) / 1000; last = ts;
  if (now >= D.duration) {{ playing = false; $('play').textContent = 'Play'; now = D.duration; }}
  const k = sceneAt(now); if (k !== current) show(k);
  let cap = '';
  D.narration.forEach((n, i) => {{
    if (now >= n.start && now < n.start + n.duration) cap = n.text;
    if (playing && !played.has(i) && now >= n.start) {{ played.add(i); if (audio[i]) {{ audio[i].currentTime = 0; audio[i].play().catch(() => 0); }} }}
  }});
  $('cap').textContent = cap; $('time').textContent = fmt(now); $('seek').value = now;
  if (playing && !once) requestAnimationFrame(() => tick(false));
}}
$('play').onclick = () => {{
  playing = !playing; $('play').textContent = playing ? 'Pause' : 'Play';
  if (playing) {{ if (now >= D.duration) jump(0); last = performance.now(); requestAnimationFrame(() => tick(false)); }} else stopAll();
}};
$('seek').oninput = e => jump(parseFloat(e.target.value));
const start = parseFloat((location.hash.match(/t=([0-9.]+)/) || [])[1]);
if (start > 0) jump(Math.min(start, D.duration)); else {{ show(0); tick(true); }}
</script></body></html>
"##,
        title = esc(&p.title),
        accent = esc(&p.style.accent),
        bg = esc(&p.style.background),
    )
}
