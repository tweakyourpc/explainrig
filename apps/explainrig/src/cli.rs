//! The command line. Every action in the window is also here.

use std::path::{Path, PathBuf};
use std::sync::mpsc;

use explainrig_guess::{Layout, Status};
use explainrig_runner::{Config, Event};

use crate::project;

/// println! that never panics: a closed pipe (`explainrig show … | head`) just ends the output.
macro_rules! out {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!(std::io::stdout(), $($arg)*);
    }};
}

macro_rules! eout {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!(std::io::stderr(), $($arg)*);
    }};
}

pub const HELP: &str = "explainrig: documentation in, explainer video out (rendered by Showrig)

USAGE
  explainrig [DOC|PROJECT] [--scene ID]
                                    open the review window (a .docx/.md is imported first)
  explainrig doctor [--json]        check Showrig and the capture and voice capabilities
  explainrig inspect DOC [--json]   the blocks Explainrig reads from a document
  explainrig import DOC [-o DIR] [--force]
                                    make a project from the rules' best guess
                                    (default DIR: next to DOC, named DOC.explainrig)
  explainrig show PROJECT           the storyboard as text
  explainrig approve PROJECT [--all | SCENE...]
  explainrig film PROJECT           write the Showrig film folder (and preview.html) only
  explainrig prepare PROJECT        voice and exact timings through Showrig, then re-time the pages
  explainrig render PROJECT [--force]
                                    the final render (every scene approved, unless --force)
  explainrig auto DOC [-o DIR]      import, approve every guess, prepare and render: for documents
                                    written to a template

Build commands come from ~/.config/explainrig/config.json (see `explainrig doctor`).
";

pub fn main(args: &[String]) -> i32 {
    let Some((cmd, rest)) = args.split_first() else {
        out!("{HELP}");
        return 0;
    };
    let result = match cmd.as_str() {
        "help" | "-h" | "--help" => {
            out!("{HELP}");
            Ok(())
        }
        "--version" | "version" => {
            out!("explainrig {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        "doctor" => doctor(rest),
        "inspect" => inspect(rest),
        "import" => import(rest).map(|_| ()),
        "show" => one_dir(rest).and_then(|d| show(&d)),
        "approve" => approve(rest),
        "film" => one_dir(rest).and_then(|d| film(&d)),
        "prepare" => one_dir(rest).and_then(|d| prepare(&d)),
        "render" => render(rest),
        "auto" => auto(rest),
        other => Err(format!("unknown command {other:?}; try `explainrig help`")),
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            eout!("explainrig: {e}");
            1
        }
    }
}

fn flag(rest: &[String], f: &str) -> bool {
    rest.iter().any(|a| a == f)
}

fn opt(rest: &[String], f: &str) -> Option<PathBuf> {
    rest.iter()
        .position(|a| a == f)
        .and_then(|i| rest.get(i + 1))
        .map(PathBuf::from)
}

fn positional(rest: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut skip = false;
    for a in rest {
        if skip {
            skip = false;
            continue;
        }
        if a == "-o" {
            skip = true;
            continue;
        }
        if !a.starts_with("--") {
            out.push(a.clone());
        }
    }
    out
}

fn one_dir(rest: &[String]) -> Result<PathBuf, String> {
    positional(rest)
        .first()
        .map(PathBuf::from)
        .ok_or_else(|| "which project? (a folder with explainrig.json)".to_string())
}

fn config() -> Result<Config, String> {
    Config::load()
}

fn doctor(rest: &[String]) -> Result<(), String> {
    let cfg = config()?;
    let d = explainrig_runner::doctor(&cfg);
    if flag(rest, "--json") {
        out!(
            "{}",
            serde_json::to_string_pretty(&d).map_err(|e| e.to_string())?
        );
    } else {
        if let Some(p) = Config::path() {
            out!(
                "config: {}{}",
                p.display(),
                if p.exists() {
                    ""
                } else {
                    " (not present: using defaults)"
                }
            );
        }
        out!(
            "prepare: {}\nrender:  {}",
            cfg.prepare.join(" "),
            cfg.render.join(" ")
        );
        for c in &d.checks {
            out!(
                "  {} {:<22} {}",
                if c.ok { "ok  " } else { "MISS" },
                c.name,
                c.detail
            );
            if !c.ok && !c.remedy.is_empty() {
                out!("       → {}", c.remedy);
            }
        }
        out!(
            "{}",
            if d.ready() {
                "ready"
            } else {
                "not ready: see above"
            }
        );
    }
    if d.ready() {
        Ok(())
    } else {
        Err("prerequisites missing".into())
    }
}

fn inspect(rest: &[String]) -> Result<(), String> {
    let doc = positional(rest)
        .first()
        .map(PathBuf::from)
        .ok_or("which document?")?;
    let d = explainrig_doc::read(&doc).map_err(|e| e.to_string())?;
    if flag(rest, "--json") {
        out!(
            "{}",
            serde_json::to_string_pretty(&d).map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    for (i, b) in d.blocks.iter().enumerate() {
        let kind = format!("{:?}", b.kind).to_lowercase();
        let mark = format!(
            "{}{}{}",
            if b.ordered { "#" } else { " " },
            if b.all_bold { "B" } else { " " },
            b.list
                .as_deref()
                .map(|l| format!("L{l}"))
                .unwrap_or_default()
        );
        let text: String = b.text.chars().take(90).collect();
        out!(
            "{i:>3} {kind:<9} {:>1} {mark:<6} {text}{}",
            b.level,
            b.image
                .as_deref()
                .map(|i| format!(" [{i}]"))
                .unwrap_or_default()
        );
    }
    for img in &d.images {
        out!("    {} {}x{} {}", img.id, img.width, img.height, img.ext);
    }
    Ok(())
}

fn import(rest: &[String]) -> Result<PathBuf, String> {
    let doc = positional(rest)
        .first()
        .map(PathBuf::from)
        .ok_or("which document?")?;
    let dir = opt(rest, "-o").unwrap_or_else(|| project::default_dir(&doc));
    let p = project::import(&doc, &dir, flag(rest, "--force"))?;
    out!(
        "{}: {} ({}), {} scenes, {} dropped",
        dir.display(),
        p.title,
        p.doc_type.label(),
        p.scenes.len(),
        p.dropped.len()
    );
    Ok(dir)
}

fn show(dir: &Path) -> Result<(), String> {
    let p = project::load(dir)?;
    let t = project::timing(dir, &p);
    out!(
        "{} · {} · {}",
        p.title,
        p.doc_type.label(),
        if t.measured {
            "measured timing"
        } else {
            "estimated timing"
        }
    );
    for s in &p.scores {
        out!(
            "  score {:<14} {:>3}  {}",
            s.doc_type.label(),
            s.score,
            s.reasons.join("; ")
        );
    }
    for s in &p.scenes {
        let st = t.scenes.get(&s.id).map(|x| x.film_duration).unwrap_or(0.0);
        let status = match s.status {
            Status::Guess => "guess",
            Status::Edited => "edited",
            Status::Approved => "APPROVED",
        };
        out!(
            "\n[{}] {} · {} · {:.1}s · {status}",
            s.id,
            s.title,
            s.layout.label(),
            st
        );
        if let Some(over) =
            explainrig_guess::text::title_overflow(&s.title, explainrig_film::is_title(s))
        {
            out!("    ⚠ title about {over}% too wide: Showrig will cut it off");
        }
        if !s.subtitle.is_empty() {
            out!("    subtitle: {}", s.subtitle);
        }
        if let Some(img) = &s.image {
            out!("    picture: {img}");
        }
        for l in &s.lines {
            out!("    {} {}", if l.draft { "draft" } else { "     " }, l.text);
        }
        if s.layout != Layout::Title {
            for pt in &s.points {
                out!(
                    "      · {:?} {}{}",
                    pt.tone,
                    pt.head,
                    if pt.sub.is_empty() {
                        String::new()
                    } else {
                        format!(" / {}", pt.sub)
                    }
                );
            }
        }
        for w in &s.why {
            out!("    why: {w}");
        }
    }
    for d in &p.dropped {
        out!("\ndropped: {} ({})", d.name, d.why);
    }
    Ok(())
}

fn approve(rest: &[String]) -> Result<(), String> {
    let pos = positional(rest);
    let (dir, ids) = pos.split_first().ok_or("which project?")?;
    let dir = PathBuf::from(dir);
    let mut p = project::load(&dir)?;
    if flag(rest, "--all") {
        project::approve_all(&mut p);
    } else {
        for id in ids {
            let s = p
                .scenes
                .iter_mut()
                .find(|s| &s.id == id)
                .ok_or_else(|| format!("no scene {id:?}"))?;
            s.status = Status::Approved;
        }
    }
    project::save(&dir, &p)?;
    let n = p
        .scenes
        .iter()
        .filter(|s| s.status == Status::Approved)
        .count();
    out!("{n}/{} scenes approved", p.scenes.len());
    Ok(())
}

fn film(dir: &Path) -> Result<(), String> {
    let p = project::load(dir)?;
    let w = project::write(dir, &p)?;
    for warning in &w.warnings {
        eout!("warning: {warning}");
    }
    out!(
        "{}\n{} pages\n{}",
        w.storyboard.display(),
        w.pages.len(),
        w.preview.display()
    );
    Ok(())
}

/// Run a long job on this thread, printing its output as it comes.
fn stream<T>(
    job: impl FnOnce(&mpsc::Sender<Event>) -> Result<T, String> + Send + 'static,
) -> Result<T, String>
where
    T: Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    let worker = std::thread::spawn(move || job(&tx));
    for ev in rx {
        match ev {
            Event::Line(l) => out!("{l}"),
            Event::Output(p) => out!("video: {}", p.display()),
            Event::Done { .. } => {}
        }
    }
    worker
        .join()
        .map_err(|_| "the job stopped unexpectedly".to_string())?
}

fn prepare(dir: &Path) -> Result<(), String> {
    let p = project::load(dir)?;
    let cfg = config()?;
    let d = dir.to_path_buf();
    let t = stream(move |tx| project::prepare(&d, &p, &cfg, tx))?;
    out!(
        "preview: {}",
        project::film_dir(dir).join("preview.html").display()
    );
    out!("{:.1} s with the real voice", t.duration);
    Ok(())
}

fn render(rest: &[String]) -> Result<(), String> {
    let dir = one_dir(rest)?;
    let p = project::load(&dir)?;
    let cfg = config()?;
    let force = flag(rest, "--force");
    let d = dir.clone();
    stream(move |tx| project::render(&d, &p, &cfg, tx, force))?;
    if let Some(v) = explainrig_runner::local_output(&project::film_dir(&dir)) {
        out!("video: {}", v.display());
    }
    Ok(())
}

fn auto(rest: &[String]) -> Result<(), String> {
    let dir = import(rest)?;
    let mut p = project::load(&dir)?;
    project::approve_all(&mut p);
    project::save(&dir, &p)?;
    render(&[dir.display().to_string()])
}
