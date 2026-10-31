//! The Showrig contract: write a film folder Showrig can render from an Explainrig project.
//!
//! ```text
//! film/
//!   showrig.json          the storyboard (only keys proven on Showrig 0.1 through 0.3)
//!   pages/<scene>.html    one animated page per page scene, recorded by Showrig
//!   pages/img/…           the pictures those pages show
//!   preview.html          the timed slideshow: pages, captions and (after prepare) the real voice
//! ```

pub mod pages;
mod preview;
pub mod timing;

use std::path::{Path, PathBuf};

use explainrig_guess::{Layout, Project, SceneKind, lexicon};
use serde_json::{Value, json};

pub use timing::Timing;

/// What [`write_film`] wrote.
#[derive(Debug, Default)]
pub struct Written {
    pub storyboard: PathBuf,
    pub pages: Vec<PathBuf>,
    pub preview: PathBuf,
    pub warnings: Vec<String>,
}

/// True for scenes Showrig draws itself (title cards); false for scenes Explainrig makes a page for.
pub fn is_title(s: &explainrig_guess::Scene) -> bool {
    s.layout == Layout::Title || matches!(s.kind, SceneKind::Intro | SceneKind::Outro)
}

/// The storyboard for a project, with pages captured for `timing`.
pub fn storyboard(p: &Project, t: &Timing) -> Value {
    let lines: Vec<&str> = p
        .scenes
        .iter()
        .flat_map(|s| s.lines.iter().map(|l| l.text.as_str()))
        .collect();
    let mut say = serde_json::Map::new();
    for (k, v) in lexicon::say(&lines, &p.say) {
        say.insert(k, Value::String(v));
    }
    let scenes: Vec<Value> = p
        .scenes
        .iter()
        .map(|s| {
            let lines: Vec<&str> = s.lines.iter().map(|l| l.text.as_str()).collect();
            if is_title(s) {
                let kind = match s.kind {
                    SceneKind::Intro => "intro",
                    SceneKind::Outro => "outro",
                    SceneKind::Segment | SceneKind::Card => "card",
                };
                json!({"kind": kind, "id": s.id, "title": s.title, "tagline": s.subtitle, "lines": lines})
            } else {
                let wait = t.capture_seconds(&s.id);
                json!({
                    "kind": "segment",
                    "id": s.id,
                    "title": s.title,
                    "subtitle": s.subtitle,
                    "duration": wait,
                    "shots": [{
                        "capture": {"url": format!("pages/{}.html", s.id), "scale": 1.25, "actions": [format!("wait {wait:.1}")]},
                        "provenance": {"licence": "own work", "credit": "Explainrig page from the source document"},
                        "in": 0.0,
                        "weight": 1.0,
                        "fit": "fit"
                    }],
                    "lines": lines,
                    "tagline": "",
                    "color": p.style.accent,
                })
            }
        })
        .collect();
    json!({
        "showrig": 1,
        "title": p.title,
        "kind": "explainer",
        "format": {"width": 1920, "height": 1080, "fps": 30},
        "style": {
            "accent": p.style.accent,
            "background": p.style.background,
            "titles": "rise",
            "transitions": "simple",
            "pushIn": 0,
            "showcase3d": false,
            "quality": "final",
            "titleHold": null
        },
        "voice": {"source": "tts", "engine": "kokoro", "voice": p.style.voice, "speed": 1.0, "say": say},
        "music": {"source": "procedural", "mood": p.style.music, "file": null, "volumeDb": -22},
        "captions": {"mode": "burn"},
        "scenes": scenes,
        "credits": {"enabled": false, "lines": []},
        "qc": {"asr": "warn"},
        "drive": {"folder": "Tutorials"}
    })
}

/// Write (or refresh) the film folder. Pictures are copied from `project_dir`.
pub fn write_film(
    p: &Project,
    project_dir: &Path,
    film_dir: &Path,
    t: &Timing,
) -> std::io::Result<Written> {
    let pages_dir = film_dir.join("pages");
    let img_dir = pages_dir.join("img");
    std::fs::create_dir_all(&img_dir)?;
    let mut out = Written::default();

    let file_name = |file: &str, id: &str| {
        Path::new(file)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| id.to_string())
    };
    let mut copied: Vec<String> = Vec::new();
    for s in &p.scenes {
        let Some(id) = &s.image else { continue };
        if copied.contains(id) {
            continue;
        }
        let Some(img) = p.image(id) else {
            out.warnings
                .push(format!("{}: picture {id} is not in the project", s.id));
            continue;
        };
        let from = project_dir.join(&img.file);
        let to = img_dir.join(file_name(&img.file, id));
        let same = std::fs::read(&from)
            .ok()
            .zip(std::fs::read(&to).ok())
            .is_some_and(|(a, b)| a == b);
        if same {
            copied.push(id.clone());
            continue;
        }
        match std::fs::copy(&from, &to) {
            Ok(_) => copied.push(id.clone()),
            Err(e) => out
                .warnings
                .push(format!("{}: {}: {e}", s.id, from.display())),
        }
    }
    let src = |id: &str| -> Option<String> {
        let img = p.image(id)?;
        copied
            .iter()
            .any(|c| c == id)
            .then(|| format!("img/{}", file_name(&img.file, id)))
    };

    // pages of scenes that no longer exist are removed so Showrig never records a stale one
    if let Ok(entries) = std::fs::read_dir(&pages_dir) {
        for e in entries.flatten() {
            let path = e.path();
            if path.extension().is_some_and(|x| x == "html") {
                let stem = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if !p.scenes.iter().any(|s| s.id == stem && !is_title(s)) {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
    }
    for s in &p.scenes {
        if let Some(html) = pages::page(p, s, t, &src) {
            let path = pages_dir.join(format!("{}.html", s.id));
            write_if_changed(&path, &html)?;
            out.pages.push(path);
        }
    }

    for sc in &p.scenes {
        if let Some(over) = explainrig_guess::text::title_overflow(&sc.title, is_title(sc)) {
            out.warnings.push(format!(
                "{}: the title \"{}\" is about {over}% too wide and will be cut off in the video",
                sc.id, sc.title
            ));
        }
    }
    let board = storyboard(p, t);
    let text = serde_json::to_string_pretty(&board).unwrap_or_default() + "\n";
    out.storyboard = film_dir.join("showrig.json");
    write_if_changed(&out.storyboard, &text)?;

    out.preview = film_dir.join("preview.html");
    write_if_changed(&out.preview, &preview::html(p, t))?;
    Ok(out)
}

/// Showrig caches by modification time: leave unchanged files alone so they are not rebuilt.
fn write_if_changed(path: &Path, text: &str) -> std::io::Result<()> {
    if std::fs::read_to_string(path).is_ok_and(|old| old == text) {
        return Ok(());
    }
    std::fs::write(path, text)
}
