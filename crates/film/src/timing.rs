//! When each narration line starts, in seconds from the start of its scene's footage.
//!
//! Before the voice exists the times are estimated from word counts; after Showrig's `--until plan`
//! pass they come from `work/plan.json`, where narration starts and footage starts are both exact.

use std::collections::BTreeMap;
use std::path::Path;

use explainrig_guess::{Project, SceneKind};
use serde::{Deserialize, Serialize};

/// Kokoro af_heart at speed 1.0 says about 2.6 words a second.
const WORDS_PER_SECOND: f64 = 2.6;
/// Showrig's pause between lines of a scene (`plan.py` GAP) and its lead-in before a segment's first line.
const GAP: f64 = 0.5;
const LEAD: f64 = 0.8;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SceneTiming {
    /// Start of each line, seconds from the scene's footage start.
    pub lines: Vec<f64>,
    /// When the last line ends, same clock.
    pub speech_end: f64,
    /// Where the scene starts in the whole film.
    pub film_start: f64,
    /// How long the scene runs in the film.
    pub film_duration: f64,
    /// How long the page was recorded for when the timing was measured. Re-timed pages keep this
    /// length so the film keeps exactly the measured layout (and the preview stays true).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Timing {
    /// true when the times come from Showrig's plan (real voice), false for estimates.
    pub measured: bool,
    pub scenes: BTreeMap<String, SceneTiming>,
    /// Every narration line: (scene id, film start, duration, text, audio file relative to the film).
    pub narration: Vec<Narration>,
    pub duration: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Narration {
    pub scene: String,
    pub start: f64,
    pub duration: f64,
    pub text: String,
    #[serde(default)]
    pub audio: String,
}

pub fn estimate_line(text: &str) -> f64 {
    let words = text.split_whitespace().count() as f64;
    (words / WORDS_PER_SECOND + 0.3).max(1.0)
}

impl Timing {
    /// Word-count estimates for every scene.
    pub fn estimate(p: &Project) -> Timing {
        let mut t = Timing::default();
        let mut clock = 0.0;
        for s in &p.scenes {
            let lead = if s.kind == SceneKind::Intro {
                0.5
            } else {
                LEAD
            };
            let mut at = lead;
            let mut lines = Vec::new();
            for l in &s.lines {
                lines.push(round(at));
                let d = estimate_line(&l.text);
                t.narration.push(Narration {
                    scene: s.id.clone(),
                    start: round(clock + at),
                    duration: round(d),
                    text: l.text.clone(),
                    audio: String::new(),
                });
                at += d + GAP;
            }
            let speech_end = if s.lines.is_empty() { lead } else { at - GAP };
            let dur = (speech_end + 0.9).max(4.0);
            t.scenes.insert(
                s.id.clone(),
                SceneTiming {
                    lines,
                    speech_end: round(speech_end),
                    film_start: round(clock),
                    film_duration: round(dur),
                    capture: None,
                },
            );
            clock += dur;
        }
        t.duration = round(clock);
        t
    }

    /// Exact times from Showrig's `work/plan.json` for the film in `film_dir`.
    pub fn from_plan(film_dir: &Path) -> Result<Timing, String> {
        let path = film_dir.join("work").join("plan.json");
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let plan: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        let num = |v: &serde_json::Value, k: &str| {
            v.get(k).and_then(serde_json::Value::as_f64).unwrap_or(0.0)
        };
        let mut t = Timing {
            measured: true,
            duration: num(&plan, "duration"),
            ..Default::default()
        };

        // each scene's span in the film, and where its footage (or backdrop) starts
        let mut span: BTreeMap<String, (f64, f64, Option<f64>)> = BTreeMap::new();
        for v in plan
            .get("video")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
        {
            let Some(scene) = v.get("scene").and_then(|s| s.as_str()) else {
                continue;
            };
            let kind = v.get("kind").and_then(|s| s.as_str()).unwrap_or("");
            let (start, dur) = (num(v, "start"), num(v, "duration"));
            let e = span
                .entry(scene.to_string())
                .or_insert((start, start + dur, None));
            e.0 = e.0.min(start);
            e.1 = e.1.max(start + dur);
            if !matches!(kind, "stage" | "backdrop") {
                e.2 = Some(e.2.map_or(start, |f: f64| f.min(start)));
            }
        }
        for v in plan
            .get("narration")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
        {
            let scene = v
                .get("scene")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            let audio = film_relative(
                film_dir,
                v.get("path").and_then(|s| s.as_str()).unwrap_or(""),
            );
            t.narration.push(Narration {
                scene,
                start: num(v, "start"),
                duration: num(v, "duration"),
                text: v
                    .get("text")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string(),
                audio,
            });
        }
        // the recording length each page had in this plan, from the storyboard it was built with
        let board: serde_json::Value = std::fs::read_to_string(film_dir.join("showrig.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        let capture_of = |id: &str| -> Option<f64> {
            board
                .get("scenes")?
                .as_array()?
                .iter()
                .find(|s| s.get("id").and_then(|v| v.as_str()) == Some(id))?
                .get("duration")?
                .as_f64()
        };
        for (scene, (start, end, footage)) in &span {
            let origin = footage.unwrap_or(*start);
            let mine: Vec<&Narration> = t.narration.iter().filter(|n| &n.scene == scene).collect();
            let lines: Vec<f64> = mine.iter().map(|n| round(n.start - origin)).collect();
            let speech_end = mine
                .last()
                .map(|n| round(n.start + n.duration - origin))
                .unwrap_or(0.0);
            t.scenes.insert(
                scene.clone(),
                SceneTiming {
                    lines,
                    speech_end,
                    film_start: round(*start),
                    film_duration: round(end - start),
                    capture: capture_of(scene),
                },
            );
        }
        Ok(t)
    }

    /// Start of line `n` in a scene (seconds from footage start), falling back to the last known line.
    pub fn line(&self, scene: &str, n: usize) -> f64 {
        let Some(s) = self.scenes.get(scene) else {
            return LEAD;
        };
        s.lines
            .get(n)
            .or_else(|| s.lines.last())
            .copied()
            .unwrap_or(LEAD)
    }

    /// How long a page should record: through the last line, plus a short hold.
    pub fn capture_seconds(&self, scene: &str) -> f64 {
        self.scenes
            .get(scene)
            .map(|s| s.capture.unwrap_or_else(|| round(s.speech_end + 1.2)))
            .unwrap_or(6.0)
            .max(4.0)
    }
}

impl Timing {
    /// The same timing with every voice path made relative to `film_dir` (timing saved by an
    /// older Explainrig kept Showrig's absolute paths).
    pub fn relative_audio(mut self, film_dir: &Path) -> Timing {
        for n in &mut self.narration {
            n.audio = film_relative(film_dir, &n.audio);
        }
        self
    }
}

fn round(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

/// A voice file's path relative to the film folder, so the preview still plays after the project
/// is moved, copied to another computer or served over HTTP. Showrig writes absolute paths (on a
/// remote worker, paths on that worker), and its voice files always live under `work/`.
fn film_relative(film_dir: &Path, audio: &str) -> String {
    let path = Path::new(audio);
    if path.is_relative() {
        return audio.to_string();
    }
    let film = std::fs::canonicalize(film_dir).unwrap_or_else(|_| film_dir.to_path_buf());
    if let Ok(rest) = path
        .strip_prefix(&film)
        .or_else(|_| path.strip_prefix(film_dir))
    {
        return rest.to_string_lossy().replace('\\', "/");
    }
    let parts: Vec<_> = path.components().collect();
    match parts.iter().rposition(|c| c.as_os_str() == "work") {
        Some(i) => parts
            .get(i..)
            .unwrap_or_default()
            .iter()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/"),
        None => audio.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::film_relative;
    use std::path::Path;

    #[test]
    fn voice_paths_become_relative_to_the_film() {
        let film = Path::new("/nowhere/demo.explainrig/film");
        assert_eq!(
            film_relative(
                film,
                "/nowhere/demo.explainrig/film/work/voice/intro-01.wav"
            ),
            "work/voice/intro-01.wav"
        );
        assert_eq!(
            film_relative(film, "/srv/worker/inputs/job-7/film/work/voice/a.wav"),
            "work/voice/a.wav"
        );
        assert_eq!(film_relative(film, "work/voice/a.wav"), "work/voice/a.wav");
    }
}
