//! The review window: open a document, read the guess, edit and approve each scene, preview the
//! timed slideshow, then render. Everything here is also a CLI command.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText, Sense, Stroke, StrokeKind, TextureHandle, Vec2};
use explainrig_doc::Document;
use explainrig_film::Timing;
use explainrig_guess::{
    Highlight, Layout, Line, Point, Project, Scene, SceneKind, Status, Tone, lexicon, text,
};
use explainrig_runner::{Config, Doctor, Event};

use crate::project;

pub fn run(start: Option<PathBuf>, scene: Option<String>) -> i32 {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Explainrig")
            .with_inner_size([1360.0, 860.0])
            .with_min_inner_size([900.0, 560.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    let result = eframe::run_native(
        "Explainrig",
        options,
        Box::new(move |cc| {
            let mut app = App::new(&cc.egui_ctx);
            if let Some(p) = start {
                app.open(&p);
            }
            if let (Some(id), Some(p)) = (scene, &app.project) {
                app.selected = p.scenes.iter().position(|s| s.id == id).unwrap_or(0);
            }
            Ok(Box::new(app))
        }),
    );
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!(
                "explainrig: the window could not open ({e}); the commands still work, see `explainrig help`"
            );
            1
        }
    }
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Tab {
    Why,
    Say,
    Dropped,
    Log,
}

struct Job {
    name: &'static str,
    events: Receiver<Event>,
    result: Receiver<Result<(), String>>,
    started: Instant,
}

struct App {
    ctx: egui::Context,
    dir: Option<PathBuf>,
    project: Option<Project>,
    doc: Option<Document>,
    timing: Timing,
    selected: usize,
    dirty: bool,
    last_change: Instant,
    textures: HashMap<String, TextureHandle>,
    open_path: String,
    log: Vec<String>,
    job: Option<Job>,
    doctor: Option<Doctor>,
    doctor_rx: Option<Receiver<Doctor>>,
    config: Result<Config, String>,
    tab: Tab,
    message: String,
    drag_from: Option<egui::Pos2>,
    new_say: (String, String),
    video: Option<PathBuf>,
    show_doctor: bool,
}

impl App {
    fn new(ctx: &egui::Context) -> Self {
        let config = Config::load();
        let mut app = App {
            ctx: ctx.clone(),
            dir: None,
            project: None,
            doc: None,
            timing: Timing::default(),
            selected: 0,
            dirty: false,
            last_change: Instant::now(),
            textures: HashMap::new(),
            open_path: String::new(),
            log: Vec::new(),
            job: None,
            doctor: None,
            doctor_rx: None,
            config,
            tab: Tab::Why,
            message: "Drop a .docx or .md file on the window, or type its path above.".into(),
            drag_from: None,
            new_say: (String::new(), String::new()),
            video: None,
            show_doctor: false,
        };
        app.check_prerequisites();
        app
    }

    fn check_prerequisites(&mut self) {
        let Ok(cfg) = self.config.clone() else { return };
        let (tx, rx) = mpsc::channel();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(explainrig_runner::doctor(&cfg));
            ctx.request_repaint();
        });
        self.doctor_rx = Some(rx);
    }

    /// Open a project folder, or import a document (opening its project when one exists already).
    fn open(&mut self, path: &Path) {
        self.save_now();
        let is_doc = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "docx" | "md" | "markdown"));
        let dir = if is_doc {
            project::default_dir(path)
        } else {
            path.to_path_buf()
        };
        let result = if is_doc && !dir.join(project::FILE).exists() {
            project::import(path, &dir, false).map(|p| (p, "imported"))
        } else {
            project::load(&dir).map(|p| (p, "opened"))
        };
        match result {
            Ok((p, how)) => {
                self.message = format!(
                    "{how} {} · {} · {} scenes{}",
                    dir.display(),
                    p.doc_type.label(),
                    p.scenes.len(),
                    if is_doc && how == "opened" {
                        " (the existing project for this document; your edits are kept)"
                    } else {
                        ""
                    }
                );
                self.doc = explainrig_doc::read(Path::new(&p.source.path)).ok();
                self.timing = project::timing(&dir, &p);
                self.textures.clear();
                self.project = Some(p);
                self.dir = Some(dir);
                self.selected = 0;
                self.video = None;
            }
            Err(e) => self.message = format!("could not open {}: {e}", path.display()),
        }
    }

    fn save_now(&mut self) {
        if !self.dirty {
            return;
        }
        if let (Some(dir), Some(p)) = (&self.dir, &self.project) {
            match project::save(dir, p) {
                Ok(()) => {
                    self.dirty = false;
                    self.timing = project::timing(dir, p);
                }
                Err(e) => self.message = format!("not saved: {e}"),
            }
        }
    }

    fn changed(&mut self) {
        self.dirty = true;
        self.last_change = Instant::now();
    }

    fn texture(&mut self, id: &str) -> Option<TextureHandle> {
        if let Some(t) = self.textures.get(id) {
            return Some(t.clone());
        }
        let (dir, p) = (self.dir.as_ref()?, self.project.as_ref()?);
        let img = p.image(id)?;
        let bytes = std::fs::read(dir.join(&img.file)).ok()?;
        let decoded = image::load_from_memory(&bytes).ok()?.to_rgba8();
        let size = [decoded.width() as usize, decoded.height() as usize];
        let color = egui::ColorImage::from_rgba_unmultiplied(size, decoded.as_raw());
        let tex = self
            .ctx
            .load_texture(id, color, egui::TextureOptions::LINEAR);
        self.textures.insert(id.to_string(), tex.clone());
        Some(tex)
    }

    fn start_job(
        &mut self,
        name: &'static str,
        work: impl FnOnce(&mpsc::Sender<Event>) -> Result<(), String> + Send + 'static,
    ) {
        if self.job.is_some() {
            return;
        }
        self.save_now();
        let (etx, erx) = mpsc::channel();
        let (rtx, rrx) = mpsc::channel();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&etx)))
                .unwrap_or_else(|_| Err("the job stopped unexpectedly".into()));
            let _ = rtx.send(r);
            ctx.request_repaint();
        });
        self.log.push(format!("── {name} ──"));
        self.tab = Tab::Log;
        self.job = Some(Job {
            name,
            events: erx,
            result: rrx,
            started: Instant::now(),
        });
    }

    fn poll(&mut self) {
        if let Some(rx) = &self.doctor_rx
            && let Ok(d) = rx.try_recv()
        {
            self.doctor = Some(d);
            self.doctor_rx = None;
        }
        let mut finished: Option<(&'static str, Result<(), String>)> = None;
        if let Some(job) = &self.job {
            while let Ok(ev) = job.events.try_recv() {
                match ev {
                    Event::Line(l) => self.log.push(l),
                    Event::Output(p) => self.video = Some(p),
                    Event::Done { .. } => {}
                }
            }
            if let Ok(r) = job.result.try_recv() {
                finished = Some((job.name, r));
            } else {
                self.ctx.request_repaint_after(Duration::from_millis(200));
            }
        }
        if self.log.len() > 4000 {
            let cut = self.log.len() - 4000;
            self.log.drain(..cut);
        }
        if let Some((name, r)) = finished {
            self.job = None;
            match r {
                Ok(()) => {
                    self.message = format!("{name}: done");
                    if let (Some(dir), Some(p)) = (&self.dir, &self.project) {
                        self.timing = project::timing(dir, p);
                        if self.video.is_none() {
                            self.video = explainrig_runner::local_output(&project::film_dir(dir));
                        }
                    }
                    if name == "Render"
                        && let Some(v) = &self.video
                    {
                        self.message = format!("rendered: {}", v.display());
                    }
                }
                Err(e) => self.message = format!("{name} failed: {e}"),
            }
        }
        if self.dirty && self.last_change.elapsed() > Duration::from_millis(700) {
            self.save_now();
        } else if self.dirty {
            self.ctx.request_repaint_after(Duration::from_millis(300));
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Explainrig");
            ui.separator();
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.open_path)
                    .hint_text(".docx, .md or a project folder")
                    .desired_width(320.0),
            );
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (ui.button("Open").clicked() || enter) && !self.open_path.trim().is_empty() {
                let path = PathBuf::from(self.open_path.trim());
                self.open(&path);
            }
            ui.separator();
            let doctor_text = match &self.doctor {
                None if self.config.is_err() => {
                    RichText::new("⚠ config error").color(Color32::LIGHT_RED)
                }
                None => RichText::new("… checking").color(Color32::GRAY),
                Some(d) if d.ready() => {
                    RichText::new("✔ ready").color(Color32::from_rgb(80, 200, 120))
                }
                Some(_) => RichText::new("⚠ missing prerequisites").color(Color32::LIGHT_RED),
            };
            if ui
                .add(egui::Button::new(doctor_text).frame(false))
                .on_hover_text("Prerequisites: click for details")
                .clicked()
            {
                self.show_doctor = !self.show_doctor;
            }
            if let Some(job) = &self.job {
                ui.separator();
                ui.spinner();
                ui.label(format!(
                    "{} · {}s",
                    job.name,
                    job.started.elapsed().as_secs()
                ));
            }
        });
        let Some(p) = &self.project else { return };
        let approved = p
            .scenes
            .iter()
            .filter(|s| s.status == Status::Approved)
            .count();
        let all = p.scenes.len();
        let busy = self.job.is_some();
        let ready = p.approved();
        let mut action: Option<&'static str> = None;
        ui.horizontal(|ui| {
            ui.label(RichText::new(&p.title).strong().size(17.0));
            ui.label(RichText::new(format!("· {}", p.doc_type.label())))
                .on_hover_text(
                    p.scores
                        .iter()
                        .map(|s| {
                            format!(
                                "{} {}: {}",
                                s.doc_type.label(),
                                s.score,
                                s.reasons.join("; ")
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
            ui.label(format!(
                "· {:.0} s {} · {approved}/{all} approved",
                self.timing.duration,
                if self.timing.measured {
                    "measured"
                } else {
                    "estimated"
                }
            ));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(
                        !busy && ready,
                        egui::Button::new(RichText::new("Render").strong()),
                    )
                    .on_disabled_hover_text("Approve every scene first")
                    .clicked()
                {
                    action = Some("render");
                }
                if ui
                    .add_enabled(!busy, egui::Button::new("Prepare"))
                    .on_hover_text(
                        "Generate the voice and measure the exact timings (Showrig, pass 1)",
                    )
                    .clicked()
                {
                    action = Some("prepare");
                }
                if ui
                    .add_enabled(!busy, egui::Button::new("Preview"))
                    .on_hover_text("Open the timed slideshow in your browser")
                    .clicked()
                {
                    action = Some("preview");
                }
                if ui
                    .add_enabled(!busy && approved < all, egui::Button::new("Approve all"))
                    .clicked()
                {
                    action = Some("approve-all");
                }
                if let Some(v) = &self.video
                    && ui
                        .button("Play video")
                        .on_hover_text(v.display().to_string())
                        .clicked()
                {
                    action = Some("play");
                }
            });
        });
        match action {
            Some("render") => self.render(),
            Some("prepare") => self.prepare(),
            Some("preview") => self.preview(),
            Some("approve-all") => {
                if let Some(p) = self.project.as_mut() {
                    project::approve_all(p);
                }
                self.changed();
            }
            Some("play") => {
                if let Some(v) = self.video.clone() {
                    open_externally(&v, &mut self.message);
                }
            }
            _ => {}
        }
    }

    fn prepare(&mut self) {
        let (Some(dir), Some(p), Ok(cfg)) =
            (self.dir.clone(), self.project.clone(), self.config.clone())
        else {
            return;
        };
        self.start_job("Prepare", move |tx| {
            project::prepare(&dir, &p, &cfg, tx).map(|_| ())
        });
    }

    fn render(&mut self) {
        let (Some(dir), Some(p), Ok(cfg)) =
            (self.dir.clone(), self.project.clone(), self.config.clone())
        else {
            return;
        };
        self.video = None;
        self.start_job("Render", move |tx| {
            project::render(&dir, &p, &cfg, tx, false)
        });
    }

    fn preview(&mut self) {
        self.save_now();
        let (Some(dir), Some(p)) = (&self.dir, &self.project) else {
            return;
        };
        match project::write(dir, p) {
            Ok(w) => {
                if !self.timing.measured {
                    self.message =
                        "preview with estimated timings: Prepare gives the real voice".into();
                }
                open_externally(&w.preview, &mut self.message);
            }
            Err(e) => self.message = e,
        }
    }

    fn scene_list(&mut self, ui: &mut egui::Ui) {
        let Some(p) = self.project.as_mut() else {
            return;
        };
        let mut op: Option<(usize, &'static str)> = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (i, s) in p.scenes.iter().enumerate() {
                    let (dot, col) = match s.status {
                        Status::Guess => ("○", Color32::GRAY),
                        Status::Edited => ("✏", Color32::from_rgb(240, 180, 60)),
                        Status::Approved => ("✔", Color32::from_rgb(80, 200, 120)),
                    };
                    let dur = self
                        .timing
                        .scenes
                        .get(&s.id)
                        .map(|t| t.film_duration)
                        .unwrap_or(0.0);
                    let selected = i == self.selected;
                    let frame = egui::Frame::new()
                        .inner_margin(6.0)
                        .corner_radius(6.0)
                        .fill(if selected {
                            ui.visuals().selection.bg_fill.gamma_multiply(0.5)
                        } else {
                            ui.visuals().faint_bg_color
                        });
                    let r = frame
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(dot).color(col));
                                ui.vertical(|ui| {
                                    ui.label(RichText::new(&s.title).strong());
                                    ui.label(
                                        RichText::new(format!(
                                            "{} · {:.1} s · {} line(s)",
                                            s.layout.label(),
                                            dur,
                                            s.lines.len()
                                        ))
                                        .small()
                                        .weak(),
                                    );
                                });
                            });
                        })
                        .response
                        .interact(Sense::click());
                    if r.clicked() {
                        op = Some((i, "select"));
                    }
                    r.context_menu(|ui| {
                        if ui.button("Move up").clicked() {
                            op = Some((i, "up"));
                        }
                        if ui.button("Move down").clicked() {
                            op = Some((i, "down"));
                        }
                        if ui.button("Merge with next").clicked() {
                            op = Some((i, "merge"));
                        }
                        if ui.button("Duplicate").clicked() {
                            op = Some((i, "dup"));
                        }
                        if ui.button("Delete").clicked() {
                            op = Some((i, "delete"));
                        }
                    });
                    ui.add_space(2.0);
                }
                ui.add_space(6.0);
                if ui.button("+ Add scene").clicked() {
                    op = Some((self.selected, "add"));
                }
                ui.label(
                    RichText::new("Right-click a scene to move, merge, duplicate or delete it.")
                        .small()
                        .weak(),
                );
            });
        let Some((i, what)) = op else { return };
        let n = p.scenes.len();
        match what {
            "select" => self.selected = i,
            "up" if i > 0 => {
                p.scenes.swap(i, i - 1);
                self.selected = i - 1;
            }
            "down" if i + 1 < n => {
                p.scenes.swap(i, i + 1);
                self.selected = i + 1;
            }
            "merge" if i + 1 < n => {
                let next = p.scenes.remove(i + 1);
                if let Some(s) = p.scenes.get_mut(i) {
                    let offset = s.lines.len();
                    s.lines.extend(next.lines);
                    s.points.extend(next.points.into_iter().map(|mut pt| {
                        pt.line += offset;
                        pt
                    }));
                    s.highlights
                        .extend(next.highlights.into_iter().map(|mut h| {
                            h.line += offset;
                            h
                        }));
                    if s.image.is_none() {
                        s.image = next.image;
                    }
                    s.source.extend(next.source);
                    s.why.push(format!("merged with \"{}\"", next.title));
                    s.status = Status::Edited;
                }
            }
            "dup" => {
                if let Some(mut s) = p.scenes.get(i).cloned() {
                    s.id = p.fresh_id(&s.title);
                    s.status = Status::Edited;
                    p.scenes.insert(i + 1, s);
                    self.selected = i + 1;
                }
            }
            "delete" if n > 1 => {
                p.scenes.remove(i);
                self.selected = self.selected.min(p.scenes.len().saturating_sub(1));
            }
            "add" => {
                let id = p.fresh_id("new scene");
                let at = (i + 1).min(p.scenes.len());
                p.scenes.insert(at, blank_scene(id));
                self.selected = at;
            }
            _ => return,
        }
        if what != "select" {
            self.changed();
        }
    }

    fn editor(&mut self, ui: &mut egui::Ui) {
        let sel = self.selected;
        let image_ids: Vec<(String, String)> = self
            .project
            .as_ref()
            .map(|p| {
                p.images
                    .iter()
                    .map(|i| (i.id.clone(), format!("{} · {}×{}", i.id, i.width, i.height)))
                    .collect()
            })
            .unwrap_or_default();
        let image_id = self
            .project
            .as_ref()
            .and_then(|p| p.scenes.get(sel))
            .and_then(|s| s.image.clone());
        let tex = image_id.as_deref().and_then(|id| self.texture(id));
        let img_px = image_id
            .as_deref()
            .and_then(|id| self.project.as_ref().and_then(|p| p.image(id)))
            .map(|i| (i.width.max(1) as f32, i.height.max(1) as f32));
        let line_times = self
            .project
            .as_ref()
            .and_then(|p| p.scenes.get(sel))
            .and_then(|s| self.timing.scenes.get(&s.id))
            .map(|t| t.lines.clone())
            .unwrap_or_default();
        let Some(p) = self.project.as_mut() else {
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new("Drop a Word (.docx) or Markdown (.md) document here")
                        .size(20.0)
                        .weak(),
                )
            });
            return;
        };
        let Some(s) = p.scenes.get_mut(sel) else {
            return;
        };
        let mut changed = false;
        let mut approve = false;
        let mut drag_from = self.drag_from;

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.horizontal(|ui| {
                let (label, col) = match s.status {
                    Status::Guess => ("Guess", Color32::GRAY),
                    Status::Edited => ("Edited", Color32::from_rgb(240, 180, 60)),
                    Status::Approved => ("Approved", Color32::from_rgb(80, 200, 120)),
                };
                ui.label(RichText::new(label).color(col).strong());
                ui.label(RichText::new(format!("scene id: {}", s.id)).weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if s.status != Status::Approved && ui.button(RichText::new("✔ Approve scene").strong()).clicked() {
                        approve = true;
                    }
                });
            });
            ui.separator();
            egui::Grid::new("scene-fields").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                ui.label("Layout");
                let before = s.layout;
                egui::ComboBox::from_id_salt("layout").selected_text(s.layout.label()).show_ui(ui, |ui| {
                    for l in Layout::ALL {
                        ui.selectable_value(&mut s.layout, l, l.label());
                    }
                });
                if s.layout != before {
                    changed = true;
                    s.kind = match (s.layout, s.kind) {
                        (Layout::Title, SceneKind::Segment) => SceneKind::Card,
                        (Layout::Title, k) => k,
                        (_, SceneKind::Card) => SceneKind::Segment,
                        (_, k) => k,
                    };
                }
                ui.end_row();
                ui.label("Title");
                ui.vertical(|ui| {
                    changed |= ui.add(egui::TextEdit::singleline(&mut s.title).desired_width(f32::INFINITY)).changed();
                    if let Some(over) = text::title_overflow(&s.title, explainrig_film::is_title(s)) {
                        ui.label(RichText::new(format!("⚠ about {over}% too wide: the video will cut this title off. Shorten it; the page heading can stay long.")).small().color(Color32::from_rgb(240, 180, 60)));
                    }
                });
                ui.end_row();
                ui.label(if s.layout == Layout::Title { "Tagline" } else { "Subtitle" });
                changed |= ui.add(egui::TextEdit::singleline(&mut s.subtitle).desired_width(f32::INFINITY)).changed();
                ui.end_row();
                if s.layout != Layout::Title && !matches!(s.kind, SceneKind::Intro | SceneKind::Outro) {
                    ui.label("Page heading");
                    changed |= ui.add(egui::TextEdit::singleline(&mut s.heading).desired_width(f32::INFINITY)).changed();
                    ui.end_row();
                }
            });

            ui.add_space(8.0);
            ui.label(RichText::new("Narration").strong());
            let mut remove_line = None;
            let mut swap_line = None;
            let n_lines = s.lines.len();
            for (k, l) in s.lines.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    let at = line_times.get(k).map(|t| format!("{t:>5.1}s")).unwrap_or_else(|| "     ".into());
                    ui.label(RichText::new(at).monospace().weak());
                    if l.draft {
                        ui.label(RichText::new("draft").small().color(Color32::from_rgb(240, 180, 60)))
                            .on_hover_text("Written by the rules from the document. Edit it into your own words.");
                    }
                    let w = ui.available_width() - 70.0;
                    if ui.add(egui::TextEdit::multiline(&mut l.text).desired_rows(1).desired_width(w.max(200.0))).changed() {
                        l.draft = false;
                        changed = true;
                    }
                    if k > 0 && ui.small_button("⬆").clicked() {
                        swap_line = Some(k);
                    }
                    if ui.small_button("🗙").clicked() && n_lines > 0 {
                        remove_line = Some(k);
                    }
                });
            }
            if let Some(k) = swap_line {
                s.lines.swap(k, k - 1);
                changed = true;
            }
            if let Some(k) = remove_line {
                s.lines.remove(k);
                for pt in &mut s.points {
                    pt.line = pt.line.min(s.lines.len().saturating_sub(1));
                }
                changed = true;
            }
            if ui.button("+ Line").clicked() {
                s.lines.push(Line { text: String::new(), draft: false });
                changed = true;
            }
            let words: usize = s.lines.iter().map(|l| text::words(&l.text)).sum();
            ui.label(RichText::new(format!("{words} words · about {:.0} s spoken", words as f64 / 2.6)).small().weak());

            if s.layout != Layout::Title && !matches!(s.kind, SceneKind::Intro | SceneKind::Outro) {
                ui.add_space(8.0);
                ui.label(RichText::new(match s.layout {
                    Layout::Timeline => "Timeline stops",
                    Layout::Steps | Layout::Checklist => "Items",
                    Layout::Compare => "Columns (first two)",
                    _ => "Callouts",
                }).strong());
                let lines_n = s.lines.len().max(1);
                let mut remove_pt = None;
                for (k, pt) in s.points.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt(("tone", k)).width(70.0).selected_text(tone_label(pt.tone)).show_ui(ui, |ui| {
                            for t in [Tone::Ok, Tone::Warn, Tone::Bad, Tone::Info] {
                                changed |= ui.selectable_value(&mut pt.tone, t, tone_label(t)).changed();
                            }
                        });
                        egui::ComboBox::from_id_salt(("pline", k)).width(70.0).selected_text(format!("line {}", pt.line + 1)).show_ui(ui, |ui| {
                            for l in 0..lines_n {
                                changed |= ui.selectable_value(&mut pt.line, l, format!("line {}", l + 1)).changed();
                            }
                        }).response.on_hover_text("Appears when this narration line starts");
                        let w = (ui.available_width() - 40.0) / 2.0;
                        changed |= ui.add(egui::TextEdit::singleline(&mut pt.head).hint_text("bold line").desired_width(w)).changed();
                        changed |= ui.add(egui::TextEdit::singleline(&mut pt.sub).hint_text("smaller line").desired_width(w)).changed();
                        if ui.small_button("🗙").clicked() {
                            remove_pt = Some(k);
                        }
                    });
                }
                if let Some(k) = remove_pt {
                    s.points.remove(k);
                    changed = true;
                }
                if ui.button("+ Item").clicked() {
                    s.points.push(Point { head: String::new(), sub: String::new(), tone: Tone::Info, line: s.lines.len().saturating_sub(1) });
                    changed = true;
                }
                if s.layout == Layout::Steps {
                    ui.horizontal(|ui| {
                        ui.label("Current step");
                        let mut f = s.focus.map(|f| f + 1).unwrap_or(0);
                        if ui.add(egui::DragValue::new(&mut f).range(0..=s.points.len())).changed() {
                            s.focus = f.checked_sub(1);
                            changed = true;
                        }
                    });
                }
            }

            if s.layout == Layout::Screenshot {
                ui.add_space(8.0);
                ui.label(RichText::new("Picture and highlights").strong());
                let before = s.image.clone();
                egui::ComboBox::from_id_salt("image").selected_text(s.image.clone().unwrap_or_else(|| "none".into())).show_ui(ui, |ui| {
                    ui.selectable_value(&mut s.image, None, "none");
                    for (id, label) in &image_ids {
                        ui.selectable_value(&mut s.image, Some(id.clone()), label);
                    }
                });
                if s.image != before {
                    s.highlights.clear();
                    changed = true;
                }
                if let (Some(tex), Some((iw, ih))) = (&tex, img_px) {
                    if s.image == image_id {
                        ui.label(RichText::new("Drag on the picture to draw a highlight box.").small().weak());
                        let max_w = ui.available_width().min(560.0);
                        let scale = (max_w / iw).min(440.0 / ih);
                        let size = Vec2::new(iw * scale, ih * scale);
                        let r = ui.add(egui::Image::new((tex.id(), size)).sense(Sense::click_and_drag()));
                        let painter = ui.painter_at(r.rect);
                        let to_screen = |h: &Highlight| {
                            egui::Rect::from_min_size(
                                r.rect.min + Vec2::new(h.x as f32 * scale, h.y as f32 * scale),
                                Vec2::new(h.w as f32 * scale, h.h as f32 * scale),
                            )
                        };
                        for (k, h) in s.highlights.iter().enumerate() {
                            let rect = to_screen(h);
                            painter.rect_stroke(rect, 4.0, Stroke::new(3.0, Color32::from_rgb(243, 146, 0)), StrokeKind::Outside);
                            painter.text(rect.left_top() + Vec2::new(2.0, -14.0), egui::Align2::LEFT_TOP, format!("{}", k + 1), egui::FontId::proportional(12.0), Color32::from_rgb(243, 146, 0));
                        }
                        if r.drag_started() {
                            drag_from = r.interact_pointer_pos();
                        }
                        if let (Some(a), Some(b)) = (drag_from, r.interact_pointer_pos()) {
                            let rect = egui::Rect::from_two_pos(a, b).intersect(r.rect);
                            painter.rect_stroke(rect, 4.0, Stroke::new(2.0, Color32::WHITE), StrokeKind::Outside);
                            if r.drag_stopped() {
                                let x0 = ((rect.min.x - r.rect.min.x) / scale).max(0.0);
                                let y0 = ((rect.min.y - r.rect.min.y) / scale).max(0.0);
                                let (w, h) = (rect.width() / scale, rect.height() / scale);
                                if w > 6.0 && h > 6.0 {
                                    s.highlights.push(Highlight {
                                        x: x0 as u32,
                                        y: y0 as u32,
                                        w: w as u32,
                                        h: h as u32,
                                        line: s.lines.len().saturating_sub(1).min(s.highlights.len()),
                                        click: false,
                                    });
                                    changed = true;
                                }
                                drag_from = None;
                            }
                        }
                        if !r.dragged() && !r.drag_stopped() {
                            drag_from = None;
                        }
                    }
                    let lines_n = s.lines.len().max(1);
                    let mut remove_h = None;
                    for (k, h) in s.highlights.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(format!("box {}", k + 1));
                            egui::ComboBox::from_id_salt(("hline", k)).width(70.0).selected_text(format!("line {}", h.line + 1)).show_ui(ui, |ui| {
                                for l in 0..lines_n {
                                    changed |= ui.selectable_value(&mut h.line, l, format!("line {}", l + 1)).changed();
                                }
                            });
                            changed |= ui.checkbox(&mut h.click, "cursor clicks it").changed();
                            if ui.small_button("🗙").clicked() {
                                remove_h = Some(k);
                            }
                        });
                    }
                    if let Some(k) = remove_h {
                        s.highlights.remove(k);
                        changed = true;
                    }
                } else if s.image.is_some() {
                    ui.label(RichText::new("The picture file could not be loaded.").color(Color32::LIGHT_RED));
                }
            }
        });
        self.drag_from = drag_from;
        if approve {
            s.status = Status::Approved;
            self.changed();
        } else if changed {
            if s.status != Status::Edited {
                s.status = Status::Edited;
            }
            self.changed();
        }
    }

    fn side(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, Tab::Why, "Why");
            ui.selectable_value(&mut self.tab, Tab::Say, "Pronunciation");
            ui.selectable_value(&mut self.tab, Tab::Dropped, "Dropped");
            ui.selectable_value(&mut self.tab, Tab::Log, "Log");
        });
        ui.separator();
        match self.tab {
            Tab::Why => self.why(ui),
            Tab::Say => self.say(ui),
            Tab::Dropped => self.dropped(ui),
            Tab::Log => {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for l in &self.log {
                            ui.label(RichText::new(l).monospace().size(11.0));
                        }
                    });
            }
        }
    }

    fn why(&mut self, ui: &mut egui::Ui) {
        let Some(p) = &self.project else { return };
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if let Some(s) = p.scenes.get(self.selected) {
                    ui.label(RichText::new("Why this scene").strong());
                    for w in &s.why {
                        ui.label(format!("• {w}"));
                    }
                    if let Some(doc) = &self.doc
                        && !s.source.is_empty()
                    {
                        ui.add_space(8.0);
                        ui.label(RichText::new("From the document").strong());
                        for i in &s.source {
                            if let Some(b) = doc.blocks.get(*i)
                                && !b.text.is_empty()
                            {
                                egui::Frame::new()
                                    .inner_margin(6.0)
                                    .corner_radius(4.0)
                                    .fill(ui.visuals().faint_bg_color)
                                    .show(ui, |ui| {
                                        ui.label(RichText::new(&b.text).size(12.5));
                                    });
                            }
                        }
                    }
                }
                ui.add_space(10.0);
                ui.label(RichText::new("Document type").strong());
                for sc in &p.scores {
                    ui.label(format!("{} · {}", sc.doc_type.label(), sc.score));
                    for r in &sc.reasons {
                        ui.label(RichText::new(format!("   {r}")).small().weak());
                    }
                }
            });
    }

    fn say(&mut self, ui: &mut egui::Ui) {
        let Some(p) = self.project.as_mut() else {
            return;
        };
        let lines: Vec<String> = p
            .scenes
            .iter()
            .flat_map(|s| s.lines.iter().map(|l| l.text.clone()))
            .collect();
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let current = lexicon::say(&refs, &p.say);
        let mut changed = false;
        ui.label(
            RichText::new("How the voice says words. Captions keep the written form.")
                .small()
                .weak(),
        );
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new("say")
                    .num_columns(3)
                    .striped(true)
                    .show(ui, |ui| {
                        for (k, v) in current {
                            ui.label(RichText::new(&k).monospace());
                            let mut val = v.clone();
                            if ui
                                .add(egui::TextEdit::singleline(&mut val).desired_width(150.0))
                                .changed()
                            {
                                p.say.insert(k.clone(), val);
                                changed = true;
                            }
                            if ui
                                .small_button("🗙")
                                .on_hover_text("Say it as written")
                                .clicked()
                            {
                                p.say.insert(k.clone(), String::new());
                                changed = true;
                            }
                            ui.end_row();
                        }
                    });
                let removed: Vec<String> = p
                    .say
                    .iter()
                    .filter(|(_, v)| v.is_empty())
                    .map(|(k, _)| k.clone())
                    .collect();
                if !removed.is_empty() {
                    ui.add_space(6.0);
                    ui.label(RichText::new("Said as written").small().weak());
                    for k in removed {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&k).monospace());
                            if ui.small_button("restore").clicked() {
                                p.say.remove(&k);
                                changed = true;
                            }
                        });
                    }
                }
                ui.add_space(8.0);
                ui.label(RichText::new("Add a pronunciation").strong());
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_say.0)
                            .hint_text("written")
                            .desired_width(90.0),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_say.1)
                            .hint_text("spoken")
                            .desired_width(120.0),
                    );
                    if ui.button("Add").clicked()
                        && !self.new_say.0.trim().is_empty()
                        && !self.new_say.1.trim().is_empty()
                    {
                        p.say.insert(
                            self.new_say.0.trim().to_string(),
                            self.new_say.1.trim().to_string(),
                        );
                        self.new_say = (String::new(), String::new());
                        changed = true;
                    }
                });
            });
        if changed {
            self.changed();
        }
    }

    fn dropped(&mut self, ui: &mut egui::Ui) {
        let Some(p) = self.project.as_mut() else {
            return;
        };
        let mut restore = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if p.dropped.is_empty() {
                    ui.label(RichText::new("Nothing was left out.").weak());
                }
                for (k, d) in p.dropped.iter().enumerate() {
                    ui.label(RichText::new(&d.name).strong());
                    ui.label(RichText::new(&d.why).small().weak());
                    ui.label(RichText::new(&d.text).size(12.5));
                    if ui.button("Restore as a scene").clicked() {
                        restore = Some(k);
                    }
                    ui.separator();
                }
            });
        if let Some(k) = restore {
            let d = p.dropped.remove(k);
            let casing = text::casing_map(&d.text);
            let body = d
                .text
                .strip_prefix(&format!("{}:", d.name))
                .unwrap_or(&d.text);
            let lines: Vec<Line> = text::sentences(body)
                .iter()
                .take(3)
                .map(|s| Line::draft(text::speakable(s, &casing)))
                .collect();
            let mut s = blank_scene(p.fresh_id(&d.name));
            s.title = d.name.clone();
            s.heading = d.name.clone();
            s.points = lines
                .iter()
                .enumerate()
                .map(|(i, l)| {
                    let (head, sub) = text::condense(&l.text, 9, 12);
                    Point {
                        head,
                        sub,
                        tone: text::tone(&l.text),
                        line: i,
                    }
                })
                .collect();
            s.lines = lines;
            s.source = d.source;
            s.why = vec![format!("restored from the dropped section \"{}\"", d.name)];
            let at = 1.min(p.scenes.len());
            p.scenes.insert(at, s);
            self.selected = at;
            self.changed();
        }
    }

    fn doctor_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_doctor;
        egui::Window::new("Prerequisites").open(&mut open).default_width(520.0).show(ctx, |ui| {
            match &self.config {
                Ok(c) => {
                    if let Some(path) = Config::path() {
                        ui.label(RichText::new(format!("config: {}", path.display())).small().weak());
                    }
                    ui.label(RichText::new(format!("prepare: {}\nrender:  {}", c.prepare.join(" "), c.render.join(" "))).monospace().size(11.0));
                }
                Err(e) => {
                    ui.label(RichText::new(e).color(Color32::LIGHT_RED));
                }
            }
            ui.separator();
            match &self.doctor {
                None => {
                    ui.spinner();
                }
                Some(d) => {
                    for c in &d.checks {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(if c.ok { RichText::new("✔").color(Color32::from_rgb(80, 200, 120)) } else { RichText::new("⚠").color(Color32::LIGHT_RED) });
                            ui.label(RichText::new(&c.name).strong());
                            ui.label(&c.detail);
                        });
                        if !c.ok && !c.remedy.is_empty() {
                            ui.label(RichText::new(format!("→ {}", c.remedy)).small());
                        }
                    }
                }
            }
            ui.separator();
            ui.label(RichText::new("Explainrig never installs anything. It renders through Showrig, which needs its capture (headless browser) and voice (Kokoro) setup.").small().weak());
            if ui.button("Check again").clicked() {
                self.doctor = None;
                self.check_prerequisites();
            }
        });
        self.show_doctor = open;
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll();
        let ctx = ui.ctx().clone();
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        if let Some(path) = dropped.into_iter().next() {
            self.open(&path);
        }
        egui::Panel::top("top").show(ui, |ui| {
            ui.add_space(4.0);
            self.top_bar(ui);
            ui.add_space(2.0);
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&self.message).small());
                if self.dirty {
                    ui.label(RichText::new("· saving…").small().weak());
                }
            });
        });
        if self.project.is_some() {
            egui::Panel::left("scenes")
                .resizable(true)
                .default_size(270.0)
                .show(ui, |ui| self.scene_list(ui));
            egui::Panel::right("side")
                .resizable(true)
                .default_size(330.0)
                .show(ui, |ui| self.side(ui));
        }
        egui::CentralPanel::default().show(ui, |ui| self.editor(ui));
        self.doctor_window(&ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_now();
    }
}

fn blank_scene(id: String) -> Scene {
    Scene {
        id,
        kind: SceneKind::Segment,
        layout: Layout::Points,
        title: "New scene".into(),
        subtitle: String::new(),
        heading: String::new(),
        lines: vec![Line {
            text: String::new(),
            draft: false,
        }],
        points: Vec::new(),
        image: None,
        highlights: Vec::new(),
        focus: None,
        source: Vec::new(),
        why: vec!["added by hand".into()],
        status: Status::Edited,
    }
}

fn tone_label(t: Tone) -> &'static str {
    match t {
        Tone::Ok => "✔ good",
        Tone::Warn => "! heads-up",
        Tone::Bad => "🗙 ends",
        Tone::Info => "i info",
    }
}

/// Open a file with the desktop's default program.
fn open_externally(path: &Path, message: &mut String) {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    match std::process::Command::new(opener).arg(path).spawn() {
        Ok(_) => *message = format!("opened {}", path.display()),
        Err(e) => *message = format!("could not open {} with {opener}: {e}", path.display()),
    }
}
