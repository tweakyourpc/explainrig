//! The animated pages Showrig records, one per page scene. Each layout places the scene's points,
//! picture and highlights, and schedules them as cues at the times their narration lines start.

use std::fmt::Write as _;

use explainrig_guess::{Layout, Project, Scene, Tone};

use crate::timing::Timing;

const CSS: &str = include_str!("../assets/page.css");
const ENGINE: &str = include_str!("../assets/engine.js");

/// Page size in CSS pixels; Showrig records it at scale 1.25 for 1920×1080.
pub const PAGE_W: f64 = 1536.0;
pub const PAGE_H: f64 = 864.0;

pub fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            c => o.push(c),
        }
    }
    o
}

fn icon(t: Tone) -> (&'static str, &'static str) {
    match t {
        Tone::Ok => ("ok", "✓"),
        Tone::Warn => ("warn", "!"),
        Tone::Bad => ("bad", "✕"),
        Tone::Info => ("info", "i"),
    }
}

fn tone_class(t: Tone) -> &'static str {
    icon(t).0
}

/// A cue: (seconds, action, selector, value).
struct Cues(Vec<(f64, &'static str, String, String)>);

impl Cues {
    fn add(&mut self, t: f64, action: &'static str, sel: impl Into<String>) {
        self.0.push((t.max(0.0), action, sel.into(), String::new()));
    }
    fn value(
        &mut self,
        t: f64,
        action: &'static str,
        sel: impl Into<String>,
        v: impl Into<String>,
    ) {
        self.0.push((t.max(0.0), action, sel.into(), v.into()));
    }
    fn js(&self) -> String {
        let mut s = String::from("const CUES = [\n");
        for (t, a, sel, v) in &self.0 {
            let _ = writeln!(s, "  [{t:.2}, {}, {}, {}],", json(a), json(sel), json(v));
        }
        s.push_str("];\n");
        s
    }
}

fn json(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// Phase pills in the top bar: the intro's subtitle splits into them for timelines; otherwise none.
fn top(p: &Project) -> String {
    format!(
        "<div class=\"top\"><b>{}</b><div class=\"sp\"></div></div>",
        esc(&p.title)
    )
}

fn shell(p: &Project, s: &Scene, body: &str, cues: &Cues) -> String {
    let accent = esc(&p.style.accent);
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title><style>{CSS}\n:root{{--accent:{accent}}}</style></head><body>\n{top}\n{body}\n\
<svg id=\"cursor\" viewBox=\"0 0 24 24\"><path d=\"M4 2l15 11-6.5 1.2L16 21l-3 1.4-3.4-6.8L4 20z\" fill=\"#fff\" stroke=\"#111\" stroke-width=\"1.4\" stroke-linejoin=\"round\"/></svg>\n\
<script>\n{cues}{ENGINE}</script></body></html>\n",
        title = esc(&s.title),
        top = top(p),
        cues = cues.js(),
    )
}

fn note(id: &str, tone: Tone, head: &str, sub: &str) -> String {
    let (cls, mark) = icon(tone);
    let sub = if sub.is_empty() {
        String::new()
    } else {
        format!("<span>{}</span>", esc(sub))
    };
    format!(
        "<div class=\"note hidden\" id=\"{id}\"><div class=\"ic {cls}\">{mark}</div><div><b>{}</b>{sub}</div></div>",
        esc(head)
    )
}

/// When point `i` appears: with its line, staggered when several share a line.
fn point_times(s: &Scene, t: &Timing) -> Vec<f64> {
    let mut seen: Vec<usize> = Vec::new();
    s.points
        .iter()
        .map(|pt| {
            let k = seen.iter().filter(|l| **l == pt.line).count();
            seen.push(pt.line);
            t.line(&s.id, pt.line) + 0.3 + 0.6 * k as f64
        })
        .collect()
}

/// Write the page for a scene, or None for title scenes (Showrig draws those itself).
pub fn page(
    p: &Project,
    s: &Scene,
    t: &Timing,
    img_src: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    if crate::is_title(s) {
        return None;
    }
    let times = point_times(s, t);
    let mut cues = Cues(Vec::new());
    let body = match s.layout {
        Layout::Screenshot => screenshot(p, s, t, &times, &mut cues, img_src),
        Layout::Timeline => timeline(s, &times, &mut cues),
        Layout::Steps => steps(s, t, &mut cues),
        Layout::Compare => compare(s, &times, &mut cues),
        Layout::Checklist => checklist(s, t, &mut cues),
        Layout::Points | Layout::Title => points(s, &times, &mut cues),
    };
    Some(shell(p, s, &body, &cues))
}

fn heading(s: &Scene) -> &str {
    if s.heading.is_empty() {
        &s.title
    } else {
        &s.heading
    }
}

fn points(s: &Scene, times: &[f64], cues: &mut Cues) -> String {
    let mut body = format!(
        "<div class=\"full\" style=\"max-width:1100px;margin:0 auto\"><h2 class=\"heading\">{}</h2>",
        esc(heading(s))
    );
    for (i, pt) in s.points.iter().enumerate() {
        body.push_str(&note(&format!("n{i}"), pt.tone, &pt.head, &pt.sub));
        cues.add(
            times.get(i).copied().unwrap_or(1.0),
            "show",
            format!("#n{i}"),
        );
    }
    body.push_str("</div>");
    body
}

fn screenshot(
    p: &Project,
    s: &Scene,
    t: &Timing,
    times: &[f64],
    cues: &mut Cues,
    img_src: &dyn Fn(&str) -> Option<String>,
) -> String {
    let image = s
        .image
        .as_deref()
        .and_then(|id| p.image(id).map(|i| (id, i)));
    let Some((id, img)) = image else {
        return points(s, times, cues);
    };
    let Some(src) = img_src(id) else {
        return points(s, times, cues);
    };
    let (iw, ih) = (f64::from(img.width.max(1)), f64::from(img.height.max(1)));
    // fit inside 760×(stage height), never more than 1.6× the picture's own size
    let max_h = PAGE_H - 96.0 - 96.0;
    let scale = (760.0 / iw).min(max_h / ih).min(1.6);
    let w = (iw * scale).round();
    let mut shot = format!(
        "<div class=\"shot\" style=\"width:{w}px\"><img src=\"{}\" alt=\"{}\">",
        esc(&src),
        esc(&img.alt)
    );
    for (k, h) in s.highlights.iter().enumerate() {
        let pct = |v: u32, of: f64| f64::from(v) / of * 100.0;
        let _ = write!(
            shot,
            "<div class=\"hl\" id=\"h{k}\" style=\"left:{:.2}%;top:{:.2}%;width:{:.2}%;height:{:.2}%\"></div>",
            pct(h.x, iw),
            pct(h.y, ih),
            pct(h.w, iw),
            pct(h.h, ih)
        );
        let at = t.line(&s.id, h.line) + 0.2;
        cues.add(at, "on", format!("#h{k}"));
        if h.click {
            cues.add(at + 0.4, "click", format!("#h{k}"));
        }
        // a box gives way to the next one that starts on a later line
        if let Some(next) = s.highlights.get(k + 1)
            && next.line > h.line
        {
            cues.add(t.line(&s.id, next.line) + 0.1, "off", format!("#h{k}"));
        }
    }
    shot.push_str("</div>");
    let mut notes = format!("<div class=\"notes\"><h2>{}</h2>", esc(heading(s)));
    for (i, pt) in s.points.iter().enumerate() {
        notes.push_str(&note(&format!("n{i}"), pt.tone, &pt.head, &pt.sub));
        cues.add(
            times.get(i).copied().unwrap_or(1.0),
            "show",
            format!("#n{i}"),
        );
    }
    notes.push_str("</div>");
    format!("<div class=\"stage\">{shot}{notes}</div>")
}

fn timeline(s: &Scene, times: &[f64], cues: &mut Cues) -> String {
    let n = s.points.len().max(1);
    let mut body = format!(
        "<div class=\"full\"><div class=\"bar\"><div id=\"fill\"></div></div><div class=\"cols\" style=\"grid-template-columns:repeat({n},1fr)\">"
    );
    for (i, pt) in s.points.iter().enumerate() {
        let label = match pt.tone {
            Tone::Bad => "Ends",
            Tone::Warn => "Transition",
            Tone::Ok => "Starts",
            Tone::Info => "Date",
        };
        let _ = write!(
            body,
            "<div class=\"card {cls} hidden\" id=\"c{i}\"><small>{label}</small><div class=\"date\">{}</div><p>{}</p></div>",
            esc(&pt.head),
            esc(&pt.sub),
            cls = tone_class(pt.tone),
        );
        let at = times.get(i).copied().unwrap_or(1.0);
        cues.add(at, "show", format!("#c{i}"));
        cues.value(
            at,
            "width",
            "#fill",
            format!("{:.0}%", (i + 1) as f64 / n as f64 * 100.0),
        );
    }
    body.push_str("</div></div>");
    body
}

fn steps(s: &Scene, t: &Timing, cues: &mut Cues) -> String {
    let mut body = format!(
        "<div class=\"full\" style=\"max-width:1100px;margin:0 auto\"><h2 class=\"heading\">{}</h2><div class=\"steps\">",
        esc(heading(s))
    );
    for (i, pt) in s.points.iter().enumerate() {
        let _ = write!(
            body,
            "<div class=\"step\" id=\"s{i}\"><div class=\"ic num\">{}</div><div>{}</div></div>",
            i + 1,
            esc(&pt.head)
        );
    }
    body.push_str("</div></div>");
    if let Some(f) = s.focus {
        cues.add(t.line(&s.id, 0) + 0.2, "on", format!("#s{f}"));
    }
    body
}

fn compare(s: &Scene, times: &[f64], cues: &mut Cues) -> String {
    let mut body = format!(
        "<div class=\"full\"><h2 class=\"heading\">{}</h2><div class=\"cols\" style=\"grid-template-columns:1fr 1fr\">",
        esc(heading(s))
    );
    for (i, pt) in s.points.iter().take(2).enumerate() {
        let _ = write!(
            body,
            "<div class=\"card {} hidden\" id=\"c{i}\"><div class=\"date\" style=\"font-size:34px\">{}</div><p>{}</p></div>",
            tone_class(pt.tone),
            esc(&pt.head),
            esc(&pt.sub)
        );
        cues.add(
            times.get(i).copied().unwrap_or(1.0) + 0.8 * i as f64,
            "show",
            format!("#c{i}"),
        );
    }
    body.push_str("</div></div>");
    body
}

fn checklist(s: &Scene, t: &Timing, cues: &mut Cues) -> String {
    let mut body = format!(
        "<div class=\"full\" style=\"max-width:1100px;margin:0 auto\"><h2 class=\"heading\">{}</h2>",
        esc(heading(s))
    );
    let start = t.line(&s.id, 0) + 0.6;
    let end = t
        .scenes
        .get(&s.id)
        .map(|x| x.speech_end)
        .unwrap_or(start + 4.0);
    let n = s.points.len().max(1) as f64;
    for (i, pt) in s.points.iter().enumerate() {
        let _ = write!(
            body,
            "<div class=\"check\" id=\"k{i}\"><i>✓</i><div>{}</div></div>",
            esc(&pt.head)
        );
        cues.add(
            start + (end - start).max(1.0) * i as f64 / n,
            "on",
            format!("#k{i}"),
        );
    }
    body.push_str("</div>");
    body
}
