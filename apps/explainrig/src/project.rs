//! A project folder and what can be done to it.
//!
//! ```text
//! <name>.explainrig/
//!   explainrig.json   the storyboard, edits and approvals
//!   img/              pictures copied out of the source document
//!   timing.json       measured line times from the last prepare (valid while the lines are unchanged)
//!   film/             the Showrig film folder Explainrig writes and Showrig renders
//! ```

use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use explainrig_film::{Timing, write_film};
use explainrig_guess::{Project, Source, Status, fingerprint, guess};
use explainrig_runner::{Config, Event, run};
use serde::{Deserialize, Serialize};

pub const FILE: &str = "explainrig.json";

pub fn default_dir(doc: &Path) -> PathBuf {
    let stem = doc
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "project".into());
    doc.with_file_name(format!("{stem}.explainrig"))
}

/// Read a document and make a new project from the rules' guess. Refuses to replace an existing
/// project unless `force`, so approvals and edits are never lost by accident.
pub fn import(doc: &Path, dir: &Path, force: bool) -> Result<Project, String> {
    if dir.join(FILE).exists() && !force {
        return Err(format!(
            "{} already has a project; open it, or import again with --force to start over",
            dir.display()
        ));
    }
    let bytes = std::fs::read(doc).map_err(|e| format!("{}: {e}", doc.display()))?;
    let d = explainrig_doc::read(doc).map_err(|e| e.to_string())?;
    let abs = std::fs::canonicalize(doc).unwrap_or_else(|_| doc.to_path_buf());
    let p = guess(
        &d,
        Source {
            path: abs.display().to_string(),
            hash: fingerprint(&bytes),
        },
    );
    std::fs::create_dir_all(dir.join("img")).map_err(|e| format!("{}: {e}", dir.display()))?;
    for (img, r) in d.images.iter().zip(p.images.iter()) {
        std::fs::write(dir.join(&r.file), &img.data).map_err(|e| format!("{}: {e}", r.file))?;
    }
    let _ = std::fs::remove_file(dir.join("timing.json"));
    save(dir, &p)?;
    Ok(p)
}

pub fn load(dir: &Path) -> Result<Project, String> {
    let path = dir.join(FILE);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Save atomically: write a temporary file, then rename it over the old one.
pub fn save(dir: &Path, p: &Project) -> Result<(), String> {
    let text = serde_json::to_string_pretty(p).map_err(|e| e.to_string())? + "\n";
    let tmp = dir.join(format!(".{FILE}.tmp"));
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, dir.join(FILE)).map_err(|e| format!("{}: {e}", dir.display()))
}

/// Measured timing is tied to the exact words and voice it was measured with.
fn lines_key(p: &Project) -> String {
    let mut s = p.style.voice.clone();
    for sc in &p.scenes {
        s.push('\u{1}');
        s.push_str(&sc.id);
        for l in &sc.lines {
            s.push('\u{2}');
            s.push_str(&l.text);
        }
    }
    s.push('\u{3}');
    s.push_str(&serde_json::to_string(&p.say).unwrap_or_default());
    fingerprint(s.as_bytes())
}

#[derive(Serialize, Deserialize)]
struct SavedTiming {
    key: String,
    timing: Timing,
}

/// The measured timing when it still matches the project's lines, else an estimate.
pub fn timing(dir: &Path, p: &Project) -> Timing {
    std::fs::read_to_string(dir.join("timing.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<SavedTiming>(&t).ok())
        .filter(|s| s.key == lines_key(p))
        .map(|s| s.timing.relative_audio(&film_dir(dir)))
        .unwrap_or_else(|| Timing::estimate(p))
}

pub fn film_dir(dir: &Path) -> PathBuf {
    dir.join("film")
}

/// Write the film folder with the best timing available.
pub fn write(dir: &Path, p: &Project) -> Result<explainrig_film::Written, String> {
    let t = timing(dir, p);
    write_film(p, dir, &film_dir(dir), &t).map_err(|e| format!("writing the film: {e}"))
}

/// Pass 1: write the film, let Showrig make the voice and the plan, then re-key every page to the
/// measured line times and refresh the preview.
pub fn prepare(
    dir: &Path,
    p: &Project,
    cfg: &Config,
    tx: &Sender<Event>,
) -> Result<Timing, String> {
    let w = write(dir, p)?;
    for warning in &w.warnings {
        let _ = tx.send(Event::Line(format!("warning: {warning}")));
    }
    let film = film_dir(dir);
    if !run(&cfg.board, &film, tx) {
        return Err("Showrig did not accept the storyboard (see the log)".into());
    }
    if !run(&cfg.prepare, &film, tx) {
        return Err("the prepare pass failed (see the log)".into());
    }
    let t = Timing::from_plan(&film)?;
    let saved = SavedTiming {
        key: lines_key(p),
        timing: t.clone(),
    };
    let text = serde_json::to_string_pretty(&saved).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("timing.json"), text).map_err(|e| e.to_string())?;
    write_film(p, dir, &film, &t).map_err(|e| format!("writing the film: {e}"))?;
    let _ = tx.send(Event::Line(format!(
        "prepared: {:.1} s, {} narration lines measured; pages re-timed",
        t.duration,
        t.narration.len()
    )));
    Ok(t)
}

/// Pass 2: the final render. Every scene must be approved unless `force`.
pub fn render(
    dir: &Path,
    p: &Project,
    cfg: &Config,
    tx: &Sender<Event>,
    force: bool,
) -> Result<(), String> {
    if !p.approved() && !force {
        let waiting: Vec<&str> = p
            .scenes
            .iter()
            .filter(|s| s.status != Status::Approved)
            .map(|s| s.id.as_str())
            .collect();
        return Err(format!(
            "not every scene is approved yet: {}",
            waiting.join(", ")
        ));
    }
    if !timing(dir, p).measured {
        let _ = tx.send(Event::Line("timing is estimated: preparing first".into()));
        prepare(dir, p, cfg, tx)?;
    } else {
        write(dir, p)?;
    }
    if !run(&cfg.render, &film_dir(dir), tx) {
        return Err("the render failed (see the log)".into());
    }
    Ok(())
}

/// Every scene approved, for templated documents (`explainrig auto`).
pub fn approve_all(p: &mut Project) {
    for s in &mut p.scenes {
        s.status = Status::Approved;
    }
}
