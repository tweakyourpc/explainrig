//! Running Showrig: the prerequisite check, and the build commands.
//!
//! Explainrig never installs anything. It looks for `showrig`, asks `showrig doctor` about the
//! capture and voice capabilities explainers need, and reports what is missing with Showrig's own
//! remedy. Builds run through command templates (`{film}` is the film folder), so a machine that
//! renders elsewhere (an SSH worker, a container) only needs a different template:
//!
//! ```json
//! {"prepare": ["showrig", "make", "{film}", "--until", "plan"],
//!  "render":  ["showrig", "make", "{film}"],
//!  "board":   ["showrig", "board", "{film}"]}
//! ```
//!
//! A command may print `explainrig-output: PATH` to say where the finished video is.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub prepare: Vec<String>,
    pub render: Vec<String>,
    pub board: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        let v = |s: &[&str]| s.iter().map(|x| x.to_string()).collect();
        Self {
            prepare: v(&["showrig", "make", "{film}", "--until", "plan"]),
            render: v(&["showrig", "make", "{film}"]),
            board: v(&["showrig", "board", "{film}"]),
        }
    }
}

impl Config {
    /// `$XDG_CONFIG_HOME/explainrig/config.json` (or `~/.config/…`).
    pub fn path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("explainrig").join("config.json"))
    }

    /// The saved configuration, or the defaults. A broken file is reported, not silently replaced.
    pub fn load() -> Result<Config, String> {
        let Some(path) = Config::path() else {
            return Ok(Config::default());
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display())),
            Err(_) => Ok(Config::default()),
        }
    }
}

/// One prerequisite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    pub name: String,
    pub ok: bool,
    pub detail: String,
    /// What to do about it, when it is not ok.
    #[serde(default)]
    pub remedy: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Doctor {
    pub checks: Vec<Check>,
}

impl Doctor {
    pub fn ready(&self) -> bool {
        self.checks.iter().all(|c| c.ok)
    }
}

/// Check what an explainer render needs: Showrig itself, then its capture (headless browser) and
/// voice (Kokoro) capabilities. Uses the local `showrig`; a remote runner is checked by running it.
pub fn doctor(config: &Config) -> Doctor {
    let mut d = Doctor::default();
    let local = config.render.first().is_some_and(|c| c == "showrig");
    if !local {
        d.checks.push(Check {
            name: "runner".into(),
            ok: config.render.first().is_some_and(|c| which(c).is_some()),
            detail: format!("renders with: {}", config.render.join(" ")),
            remedy:
                "the first word of the render command must be a program on PATH or a path to one"
                    .into(),
        });
    }
    let version = Command::new("showrig").arg("--version").output();
    match version {
        Ok(o) if o.status.success() => d.checks.push(Check {
            name: "showrig".into(),
            ok: true,
            detail: String::from_utf8_lossy(&o.stdout).trim().to_string(),
            remedy: String::new(),
        }),
        _ => {
            d.checks.push(Check {
                name: "showrig".into(),
                ok: !local,
                detail: if local { "not found on PATH".into() } else { "not on this machine (renders elsewhere)".into() },
                remedy: "install Showrig (github.com/tweakyourpc/showrig): its README's Quick start, then `showrig setup`".into(),
            });
            return d;
        }
    }
    if !local {
        return d;
    }
    for profile in ["capture", "voice"] {
        let out = Command::new("showrig")
            .args(["doctor", "--profile", profile, "--json"])
            .stderr(Stdio::null())
            .output();
        let Ok(out) = out else {
            d.checks.push(Check {
                name: profile.into(),
                ok: false,
                detail: "showrig doctor did not run".into(),
                remedy: String::new(),
            });
            continue;
        };
        let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
        let caps = report
            .get("capabilities")
            .and_then(|c| c.as_array())
            .cloned()
            .unwrap_or_default();
        if caps.is_empty() {
            d.checks.push(Check {
                name: profile.into(),
                ok: out.status.success(),
                detail: format!(
                    "showrig doctor --profile {profile} exited {}",
                    out.status.code().unwrap_or(-1)
                ),
                remedy: format!(
                    "run `showrig setup` and `showrig doctor --profile {profile}` for details"
                ),
            });
            continue;
        }
        for c in caps {
            let s = |k: &str| c.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
            let state = s("state");
            let ok = matches!(state.as_str(), "ready" | "not-tested" | "found");
            let detail = [s("version"), s("path"), format!("({state})")]
                .into_iter()
                .filter(|x| !x.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            let remedy = match c.get("remedy") {
                Some(serde_json::Value::String(r)) => r.clone(),
                Some(serde_json::Value::Null) | None => String::new(),
                Some(other) => other.to_string(),
            };
            d.checks.push(Check {
                name: format!("{profile}: {}", s("id")),
                ok,
                detail,
                remedy,
            });
        }
    }
    d
}

fn which(cmd: &str) -> Option<PathBuf> {
    let p = Path::new(cmd);
    if p.components().count() > 1 {
        return p.is_file().then(|| p.to_path_buf());
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join(cmd))
        .find(|c| c.is_file())
}

/// A line of output from a running command, or how it ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Line(String),
    /// The command says where the finished video is.
    Output(PathBuf),
    Done {
        ok: bool,
        code: Option<i32>,
    },
}

/// Run a command template for a film, sending its output lines as they arrive. Blocks until it ends;
/// call it from a worker thread to keep a window responsive.
pub fn run(template: &[String], film: &Path, tx: &Sender<Event>) -> bool {
    let film_s = film.to_string_lossy().into_owned();
    let argv: Vec<String> = template
        .iter()
        .map(|a| a.replace("{film}", &film_s))
        .collect();
    let Some((prog, args)) = argv.split_first() else {
        let _ = tx.send(Event::Line("no command configured".into()));
        let _ = tx.send(Event::Done {
            ok: false,
            code: None,
        });
        return false;
    };
    let _ = tx.send(Event::Line(format!("$ {}", argv.join(" "))));
    let child = Command::new(prog)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(Event::Line(format!("could not start {prog}: {e}")));
            let _ = tx.send(Event::Done {
                ok: false,
                code: None,
            });
            return false;
        }
    };
    let mut readers = Vec::new();
    if let Some(err) = child.stderr.take() {
        let tx = tx.clone();
        readers.push(std::thread::spawn(move || {
            forward(BufReader::new(err), &tx)
        }));
    }
    if let Some(out) = child.stdout.take() {
        let tx = tx.clone();
        readers.push(std::thread::spawn(move || {
            forward(BufReader::new(out), &tx)
        }));
    }
    for r in readers {
        let _ = r.join();
    }
    let status = child.wait();
    let (ok, code) = match status {
        Ok(s) => (s.success(), s.code()),
        Err(_) => (false, None),
    };
    let _ = tx.send(Event::Done { ok, code });
    ok
}

fn forward(r: impl BufRead, tx: &Sender<Event>) {
    for line in r.lines().map_while(Result::ok) {
        if let Some(path) = line.strip_prefix("explainrig-output:") {
            let _ = tx.send(Event::Output(PathBuf::from(path.trim())));
        }
        let _ = tx.send(Event::Line(line));
    }
}

/// The newest `.mp4` in the film's `out/` folder (where a local `showrig make` leaves it).
pub fn local_output(film: &Path) -> Option<PathBuf> {
    std::fs::read_dir(film.join("out"))
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "mp4"))
        .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
}
