//! The storyboard guess: sections, document type, and a scene recipe per type.
//!
//! Every choice leaves a reason in `Scene::why` or `Score::reasons`, so the review app can say why
//! a scene exists and what it was made from.

use std::collections::BTreeMap;

use explainrig_doc::{Block, Document, Kind};

use crate::model::{
    DocType, Dropped, Layout, Line, Point, Project, SCHEMA, Scene, SceneKind, Score, Source, Style,
    Tone,
};
use crate::text::{
    self, casing_map, condense, dates, imperative, sentences, speakable, tone, words,
};

/// Section names that are context for the writer's colleagues rather than for the viewer.
const DROP_NAMES: &[&str] = &[
    "background",
    "purpose",
    "scope",
    "revision history",
    "document history",
    "table of contents",
    "contents",
    "audience",
];

const CHANGE_WORDS: &[&str] = &[
    "no longer",
    "sunset",
    "retire",
    "deprecat",
    "end of life",
    "will be removed",
    "starting",
    "beginning",
    "onward",
    "transition",
    "replace",
    "migrat",
    "going away",
    "discontinu",
    "effective",
];

/// Most content scenes a guess makes; more sections than this are still listed, as one scene each,
/// but the person is told the film is getting long.
const LONG_FILM: usize = 10;

#[derive(Debug, Clone)]
struct Section {
    name: String,
    /// The block that names the section (a heading or a bold item), when there is one.
    header: Option<usize>,
    blocks: Vec<usize>,
    why: String,
}

pub fn guess(doc: &Document, source: Source) -> Project {
    let all: String = doc
        .blocks
        .iter()
        .map(|b| b.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let casing = casing_map(&all);
    let title_block = doc
        .blocks
        .iter()
        .position(|b| b.kind == Kind::Heading && b.level <= 1);
    let raw_title = title_block
        .and_then(|i| doc.blocks.get(i))
        .map(|b| b.text.clone())
        .unwrap_or_else(|| stem(&doc.source));
    let title = clean_title(&raw_title, &casing);

    let (sections, mut dropped) = sections(doc, title_block);
    let kept: Vec<&Section> = sections.iter().collect();
    let scores = classify(doc, &kept, &raw_title);
    let doc_type = scores
        .first()
        .map(|s| s.doc_type)
        .unwrap_or(DocType::Reference);

    let mut ctx = Ctx {
        doc,
        casing: &casing,
        scenes: Vec::new(),
        dropped: Vec::new(),
    };
    ctx.intro(&title, doc_type, &kept);
    match doc_type {
        DocType::ChangeNotice => ctx.change_notice(&kept),
        DocType::Procedure => ctx.procedure(&kept),
        DocType::Faq => ctx.faq(&kept),
        DocType::Reference => ctx.reference(&kept),
    }
    ctx.outro(doc_type, &all);
    let mut scenes = ctx.scenes;
    dropped.extend(ctx.dropped);
    unique_ids(&mut scenes);
    let content = scenes
        .iter()
        .filter(|s| s.kind == SceneKind::Segment || s.kind == SceneKind::Card)
        .count();
    if content > LONG_FILM
        && let Some(first) = scenes.first_mut()
    {
        first.why.push(format!(
            "{content} content scenes: consider merging or dropping some for a short explainer"
        ));
    }
    dropped.sort_by_key(|d| d.source.first().copied().unwrap_or(0));

    let images = doc
        .images
        .iter()
        .map(|i| crate::model::ImageRef {
            id: i.id.clone(),
            file: format!(
                "img/{}.{}",
                i.id,
                if i.ext.is_empty() { "png" } else { &i.ext }
            ),
            width: i.width,
            height: i.height,
            alt: i.alt.clone(),
        })
        .collect();

    Project {
        explainrig: SCHEMA,
        title,
        source,
        doc_type,
        scores,
        style: Style::default(),
        scenes,
        dropped,
        images,
        say: BTreeMap::new(),
    }
}

fn stem(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Untitled")
        .replace(['_', '-'], " ")
}

/// "Sunsetting SMS/Voice - User Experience" → "Sunsetting SMS/Voice": generic suffixes go.
fn clean_title(t: &str, casing: &BTreeMap<String, String>) -> String {
    const GENERIC: &[&str] = &[
        "user experience",
        "overview",
        "guide",
        "faq",
        "notes",
        "draft",
        "summary",
        "information",
        "details",
        "readme",
    ];
    let mut t = t.trim().to_string();
    for sep in [" - ", " – ", " — ", ": "] {
        if let Some((head, tail)) = t.rsplit_once(sep) {
            let tail = tail.trim().to_lowercase();
            if GENERIC
                .iter()
                .any(|g| tail == *g || tail.starts_with(&format!("{g} ")))
                && !head.trim().is_empty()
            {
                t = head.trim().to_string();
            }
        }
    }
    let mut out = t.clone();
    for (caps, mixed) in casing {
        out = out
            .split(' ')
            .map(|w| if w == caps { mixed.as_str() } else { w })
            .collect::<Vec<_>>()
            .join(" ");
    }
    out
}

fn is_header_item(b: &Block) -> bool {
    matches!(b.kind, Kind::Item | Kind::Paragraph) && b.all_bold && words(&b.text) <= 30
}

fn label(b: &Block) -> Option<String> {
    let lead = b.lead.as_deref()?.trim();
    let name = lead.strip_suffix(':')?.trim();
    (words(name) <= 4 && words(&b.text) > words(lead)).then(|| name.to_string())
}

/// Split the document into sections, and drop the ones named like writer's context.
fn sections(doc: &Document, title: Option<usize>) -> (Vec<Section>, Vec<Dropped>) {
    let blocks: Vec<(usize, &Block)> = doc
        .blocks
        .iter()
        .enumerate()
        .filter(|(i, _)| Some(*i) != title)
        .collect();
    let headings: Vec<u8> = blocks
        .iter()
        .filter(|(_, b)| b.kind == Kind::Heading)
        .map(|(_, b)| b.level)
        .collect();
    let mut out: Vec<Section> = Vec::new();
    let mut open: Option<Section> = None;
    let flush = |open: &mut Option<Section>, out: &mut Vec<Section>| {
        if let Some(s) = open.take()
            && (!s.blocks.is_empty() || s.header.is_some())
        {
            out.push(s);
        }
    };

    if headings.len() >= 2 {
        let top = headings.iter().copied().min().unwrap_or(1);
        for (i, b) in &blocks {
            if b.kind == Kind::Heading && b.level == top {
                flush(&mut open, &mut out);
                open = Some(Section {
                    name: b.text.clone(),
                    header: Some(*i),
                    blocks: Vec::new(),
                    why: format!("from heading {} \"{}\"", b.level, b.text),
                });
            } else {
                open.get_or_insert_with(overview).blocks.push(*i);
            }
        }
    } else {
        let mut named_open = false;
        for (i, b) in &blocks {
            if let Some(name) = label(b) {
                // "Background: …" stands alone; what follows returns to the open section
                let held = open.take();
                out.push(Section {
                    name: name.clone(),
                    header: None,
                    blocks: vec![*i],
                    why: format!("from the label \"{name}:\""),
                });
                open = held;
            } else if is_header_item(b) {
                flush(&mut open, &mut out);
                named_open = true;
                let name = sentences(&b.text)
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| b.text.clone());
                open = Some(Section {
                    why: format!(
                        "from a bold {} \"{}\"",
                        if b.kind == Kind::Item {
                            "list item"
                        } else {
                            "line"
                        },
                        short(&name)
                    ),
                    name,
                    header: Some(*i),
                    blocks: Vec::new(),
                });
            } else {
                if open.is_none() && named_open {
                    named_open = false;
                }
                open.get_or_insert_with(overview).blocks.push(*i);
            }
        }
    }
    flush(&mut open, &mut out);
    out.sort_by_key(|s| s.header.or_else(|| s.blocks.first().copied()).unwrap_or(0));

    let mut dropped = Vec::new();
    let mut kept = Vec::new();
    for s in out {
        let lower = s.name.to_lowercase();
        if DROP_NAMES
            .iter()
            .any(|d| lower == *d || lower.starts_with(&format!("{d} ")))
        {
            let text = s
                .header
                .iter()
                .chain(s.blocks.iter())
                .filter_map(|i| doc.blocks.get(*i))
                .map(|b| b.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            dropped.push(Dropped {
                name: s.name.clone(),
                text,
                why: format!(
                    "\"{}\" is usually context for colleagues, not viewers",
                    s.name
                ),
                source: s.header.iter().chain(s.blocks.iter()).copied().collect(),
            });
        } else {
            kept.push(s);
        }
    }
    (kept, dropped)
}

fn overview() -> Section {
    Section {
        name: "Overview".into(),
        header: None,
        blocks: Vec::new(),
        why: "text before the first named section".into(),
    }
}

fn short(s: &str) -> String {
    let w: Vec<&str> = s.split_whitespace().collect();
    if w.len() > 8 {
        format!(
            "{}…",
            w.iter().take(8).copied().collect::<Vec<_>>().join(" ")
        )
    } else {
        s.to_string()
    }
}

fn classify(doc: &Document, sections: &[&Section], title: &str) -> Vec<Score> {
    let used: Vec<&Block> = sections
        .iter()
        .flat_map(|s| s.header.iter().chain(s.blocks.iter()))
        .filter_map(|i| doc.blocks.get(*i))
        .collect();
    let text: String = used
        .iter()
        .map(|b| b.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let title_l = title.to_lowercase();

    let mut keys: Vec<u32> = dates(&text).iter().map(|d| d.key).collect();
    keys.sort_unstable();
    keys.dedup();
    let change_hits: Vec<&str> = CHANGE_WORDS
        .iter()
        .copied()
        .filter(|w| text.contains(w))
        .collect();
    let ordered = used
        .iter()
        .filter(|b| b.kind == Kind::Item && b.ordered && b.level == 0)
        .count();
    let imperative_steps = used
        .iter()
        .filter(|b| b.kind == Kind::Item && b.ordered && imperative(&b.text))
        .count();
    let step_headings = used
        .iter()
        .filter(|b| b.kind == Kind::Heading && b.text.to_lowercase().starts_with("step"))
        .count();
    let questions = used
        .iter()
        .filter(|b| {
            (b.kind == Kind::Heading || b.all_bold || b.lead.is_some())
                && b.text.trim_end().ends_with('?')
        })
        .count()
        + sections
            .iter()
            .filter(|s| s.name.trim_end().ends_with('?'))
            .count();

    let mut change = Score {
        doc_type: DocType::ChangeNotice,
        score: 0,
        reasons: Vec::new(),
    };
    if !keys.is_empty() {
        change.score += 3 * keys.len().min(3) as i32;
        change
            .reasons
            .push(format!("{} date(s) mentioned", keys.len()));
    }
    if !change_hits.is_empty() {
        change.score += 2 * change_hits.len().min(5) as i32;
        change
            .reasons
            .push(format!("change words: {}", change_hits.join(", ")));
    }
    if [
        "sunset", "retir", "change", "migrat", "deprecat", "end of", "new ",
    ]
    .iter()
    .any(|w| title_l.contains(w))
    {
        change.score += 4;
        change.reasons.push("the title announces a change".into());
    }

    let mut procedure = Score {
        doc_type: DocType::Procedure,
        score: 0,
        reasons: Vec::new(),
    };
    if ordered >= 2 {
        procedure.score += ordered.min(8) as i32;
        procedure
            .reasons
            .push(format!("{ordered} numbered item(s)"));
    }
    if imperative_steps > 0 {
        procedure.score += 2 * imperative_steps.min(8) as i32;
        procedure.reasons.push(format!(
            "{imperative_steps} numbered item(s) start with an instruction"
        ));
    }
    if step_headings > 0 {
        procedure.score += 2 * step_headings as i32;
        procedure
            .reasons
            .push(format!("{step_headings} \"Step\" heading(s)"));
    }
    if title_l.starts_with("how to")
        || title_l.contains("steps")
        || title_l.contains("set up")
        || title_l.contains("install")
    {
        procedure.score += 4;
        procedure.reasons.push("the title names a task".into());
    }

    let mut faq = Score {
        doc_type: DocType::Faq,
        score: 0,
        reasons: Vec::new(),
    };
    if questions > 0 {
        faq.score = 3 * questions as i32;
        faq.reasons.push(format!("{questions} question heading(s)"));
    }
    if title_l.contains("faq") || title_l.contains("questions") {
        faq.score += 4;
        faq.reasons.push("the title says questions".into());
    }

    let reference = Score {
        doc_type: DocType::Reference,
        score: 4,
        reasons: vec!["the fallback when nothing else is clear".into()],
    };

    let mut all = vec![change, procedure, faq, reference];
    all.sort_by(|a, b| b.score.cmp(&a.score).then(a.doc_type.cmp(&b.doc_type)));
    all
}

struct Ctx<'a> {
    doc: &'a Document,
    casing: &'a BTreeMap<String, String>,
    scenes: Vec<Scene>,
    dropped: Vec<Dropped>,
}

/// A sentence from the document with where it came from.
#[derive(Debug, Clone)]
struct Sent {
    text: String,
    block: usize,
    header: bool,
    /// A later sentence of a heading: describes the group rather than telling the viewer anything.
    aside: bool,
}

impl Ctx<'_> {
    fn block(&self, i: usize) -> Option<&Block> {
        self.doc.blocks.get(i)
    }

    fn drop_section(&mut self, s: &Section, why: &str) {
        let source: Vec<usize> = s.header.iter().chain(s.blocks.iter()).copied().collect();
        let text = source
            .iter()
            .filter_map(|i| self.block(*i))
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        self.dropped.push(Dropped {
            name: s.name.clone(),
            text,
            why: why.into(),
            source,
        });
    }

    fn sents(&self, blocks: &[usize], header: Option<usize>) -> Vec<Sent> {
        header
            .iter()
            .map(|h| (*h, true))
            .chain(blocks.iter().map(|b| (*b, false)))
            .filter_map(|(i, is_header)| self.block(i).map(|b| (i, b, is_header)))
            .filter(|(_, b, _)| matches!(b.kind, Kind::Paragraph | Kind::Item | Kind::Heading))
            .flat_map(|(i, b, is_header)| {
                let body = match (&b.lead, label(b)) {
                    (Some(lead), Some(_)) => b
                        .text
                        .strip_prefix(lead.as_str())
                        .unwrap_or(&b.text)
                        .trim()
                        .to_string(),
                    _ => b.text.clone(),
                };
                sentences(&body)
                    .into_iter()
                    .enumerate()
                    .map(move |(k, s)| Sent {
                        text: s,
                        block: i,
                        header: is_header && k == 0,
                        aside: is_header && b.all_bold && k > 0,
                    })
            })
            .filter(|s| words(&s.text) >= 3)
            .collect()
    }

    fn say(&self, s: &str) -> String {
        speakable(s, self.casing)
    }

    /// Up to `n` narration lines from these sentences, best first by the line rules, kept in
    /// document order. Long sentences are split at a clause boundary.
    fn pick(&self, sents: &[Sent], n: usize) -> Vec<(String, usize)> {
        let mut scored: Vec<(i32, usize, String)> = sents
            .iter()
            .enumerate()
            .map(|(order, s)| {
                let spoken = self.say(&s.text);
                let l = spoken.to_lowercase();
                let mut score = 0;
                if s.header {
                    score += 3;
                }
                if s.aside {
                    score -= 4;
                }
                if l.contains("you") {
                    score += 2;
                }
                if !dates(&spoken).is_empty() {
                    score += 2;
                }
                if imperative(&spoken) {
                    score += 2;
                }
                if ["required", "must", "need", "can't", "won't", "no longer"]
                    .iter()
                    .any(|w| l.contains(w))
                {
                    score += 1;
                }
                let w = words(&spoken);
                if (6..=30).contains(&w) {
                    score += 1;
                }
                if w > 40 {
                    score -= 2;
                }
                if [
                    "for reference",
                    "described below",
                    "provided below",
                    "example",
                    "have been notified",
                    "announcement",
                ]
                .iter()
                .any(|x| l.contains(x))
                {
                    score -= 3;
                }
                (score, order, spoken)
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        scored.retain(|x| x.0 >= -1);
        scored.truncate(n);
        scored.sort_by_key(|x| x.1);
        let mut out = Vec::new();
        for (_, order, s) in scored {
            for part in split_long(&s) {
                out.push((part, order));
            }
        }
        out.truncate(n + 1);
        out
    }

    fn intro(&mut self, title: &str, t: DocType, sections: &[&Section]) {
        let tagline = match t {
            DocType::ChangeNotice => "What changes, and what to do",
            DocType::Procedure => "Step by step",
            DocType::Faq => "Your questions, answered",
            DocType::Reference => "The key points",
        };
        let spoken_title = title.replace('/', " and ");
        let line = match t {
            DocType::ChangeNotice => {
                format!("{spoken_title}: here's what changes for you, and what to do.")
            }
            DocType::Procedure => {
                let l = spoken_title.to_lowercase();
                let task = if let Some(rest) =
                    spoken_title.get(7..).filter(|_| l.starts_with("how to "))
                {
                    rest.to_string()
                } else if imperative(&spoken_title) {
                    lower_first(&spoken_title)
                } else {
                    format!("use {spoken_title}")
                };
                format!("Here's how to {task}, step by step.")
            }
            DocType::Faq => format!(
                "Here are the most common questions about {}.",
                lower_first(&spoken_title)
            ),
            DocType::Reference => format!(
                "Here are the key points about {}.",
                lower_first(&spoken_title)
            ),
        };
        self.scenes.push(Scene {
            id: "intro".into(),
            kind: SceneKind::Intro,
            layout: Layout::Title,
            title: title.to_string(),
            subtitle: tagline.into(),
            heading: String::new(),
            lines: vec![Line::draft(line)],
            points: Vec::new(),
            image: None,
            highlights: Vec::new(),
            focus: None,
            source: sections
                .first()
                .and_then(|s| s.header)
                .into_iter()
                .collect(),
            why: vec![format!(
                "every film opens on its title; this one reads as a {}",
                t.label()
            )],
            status: Default::default(),
        });
    }

    fn change_notice(&mut self, sections: &[&Section]) {
        // the timeline: every sentence with a date, outside the named sections first
        let mut dated: Vec<(u32, String, Sent)> = Vec::new();
        for s in sections.iter().filter(|s| s.header.is_none()) {
            for sent in self.sents(&s.blocks, None) {
                if let Some(d) = dates(&sent.text).first() {
                    dated.push((d.key, d.label.clone(), sent));
                }
            }
        }
        dated.sort_by_key(|d| d.0);
        dated.dedup_by_key(|d| d.0);
        if !dated.is_empty() {
            let mut lines = Vec::new();
            let mut points = Vec::new();
            for (n, (_, label, sent)) in dated.iter().take(4).enumerate() {
                let spoken = self.say(&sent.text);
                let after = strip_through_date(&spoken);
                let (head, _) = condense(&after, 18, 0);
                points.push(Point {
                    head: label.clone(),
                    sub: head,
                    tone: if n == 0 { Tone::Warn } else { Tone::Bad },
                    line: n,
                });
                lines.push(Line::draft(spoken));
            }
            let labels: Vec<&str> = dated.iter().map(|d| d.1.as_str()).collect();
            self.scenes.push(Scene {
                id: "dates".into(),
                kind: SceneKind::Segment,
                layout: Layout::Timeline,
                title: if dated.len() == 1 {
                    "The date to know".into()
                } else {
                    format!("{} dates to know", number_word(dated.len()))
                },
                subtitle: labels.join(" and "),
                heading: String::new(),
                lines,
                points,
                image: None,
                highlights: Vec::new(),
                focus: None,
                source: dated.iter().map(|d| d.2.block).collect(),
                why: vec![format!(
                    "timeline: {} date(s) found ({})",
                    dated.len(),
                    labels.join(", ")
                )],
                status: Default::default(),
            });
        }
        let named: Vec<&Section> = sections
            .iter()
            .copied()
            .filter(|s| s.header.is_some())
            .collect();
        self.compare_if_parallel(&named);
        for s in named {
            self.section_scenes(s);
        }
        // undated overview sentences become a points scene when nothing else covered them
        for s in sections.iter().filter(|s| s.header.is_none()) {
            let rest: Vec<usize> = s
                .blocks
                .iter()
                .copied()
                .filter(|b| !dated.iter().any(|d| d.2.block == *b))
                .filter(|b| self.block(*b).is_some_and(|b| b.kind != Kind::Image))
                .collect();
            if rest.len() >= 2 {
                self.points_scene(
                    "What else to know",
                    &rest,
                    None,
                    "overview text without dates",
                );
            }
        }
    }

    fn procedure(&mut self, sections: &[&Section]) {
        // steps are the outer numbered items; what follows a step (pictures, notes) belongs to it
        // a step's notes end where its section ends; sections after the steps are reference material
        let mut steps: Vec<(usize, Vec<usize>)> = Vec::new();
        let mut before: Vec<usize> = Vec::new();
        let is_step = |b: &Block| {
            (b.kind == Kind::Item && b.ordered && b.level == 0)
                || (b.kind == Kind::Heading && b.text.to_lowercase().starts_with("step"))
        };
        for s in sections {
            let all: Vec<usize> = s.header.iter().chain(s.blocks.iter()).copied().collect();
            let has_steps = all.iter().any(|i| self.block(*i).is_some_and(is_step));
            if !has_steps {
                if steps.is_empty() {
                    before.extend(
                        all.iter()
                            .copied()
                            .filter(|i| self.block(*i).is_some_and(|b| b.kind != Kind::Heading)),
                    );
                } else {
                    self.drop_section(
                        s,
                        "reference material after the steps; restore it to explain it too",
                    );
                }
                continue;
            }
            let mut in_step = false;
            for i in all {
                let Some(b) = self.block(i) else { continue };
                if is_step(b) {
                    steps.push((i, Vec::new()));
                    in_step = true;
                } else if in_step && let Some((_, rest)) = steps.last_mut() {
                    rest.push(i);
                } else if b.kind != Kind::Heading {
                    before.push(i);
                }
            }
        }
        if steps.is_empty() {
            // numbered headings were not found; fall back to one scene per section
            return self.reference(sections);
        }
        let prose: Vec<usize> = before
            .iter()
            .copied()
            .filter(|i| self.block(*i).is_some_and(|b| b.kind != Kind::Image))
            .collect();
        if !prose.is_empty() {
            self.points_scene(
                "Before you start",
                &prose,
                None,
                "text before the first step",
            );
            let picture = before
                .iter()
                .find_map(|i| self.block(*i).and_then(|b| b.image.clone()));
            if let (Some(img), Some(scene)) = (picture, self.scenes.last_mut()) {
                scene.layout = Layout::Screenshot;
                scene
                    .why
                    .push(format!("shows the picture before the steps ({img})"));
                scene.image = Some(img);
            }
        }
        let heads: Vec<String> = steps
            .iter()
            .filter_map(|(i, _)| self.block(*i))
            .map(|b| {
                let first = sentences(&b.text)
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| b.text.clone());
                condense(&self.say(&first), 8, 0).0
            })
            .collect();
        for (n, (step, rest)) in steps.iter().enumerate() {
            let image = rest
                .iter()
                .filter_map(|i| self.block(*i))
                .find_map(|b| b.image.clone());
            let sents = self.sents(rest, Some(*step));
            let lines = self.pick(&sents, 3);
            let head = heads.get(n).cloned().unwrap_or_default();
            let (layout, points, why) = match &image {
                Some(img) => (
                    Layout::Screenshot,
                    self.callouts(&sents, &lines),
                    format!("step {} with a picture ({img})", n + 1),
                ),
                None => (
                    Layout::Steps,
                    heads
                        .iter()
                        .map(|h| Point {
                            head: h.clone(),
                            sub: String::new(),
                            tone: Tone::Info,
                            line: 0,
                        })
                        .collect(),
                    format!(
                        "step {} without a picture: shown in the list of all steps",
                        n + 1
                    ),
                ),
            };
            self.scenes.push(Scene {
                id: format!("step{}", n + 1),
                kind: SceneKind::Segment,
                layout,
                title: text::fit_title(&format!("{}. {}", n + 1, head), text::LOWER_THIRD_EM),
                subtitle: String::new(),
                heading: head.clone(),
                lines: lines.into_iter().map(|(t, _)| Line::draft(t)).collect(),
                points,
                image,
                highlights: Vec::new(),
                focus: (layout == Layout::Steps).then_some(n),
                source: std::iter::once(*step).chain(rest.iter().copied()).collect(),
                why: vec![why],
                status: Default::default(),
            });
        }
        if (3..=6).contains(&heads.len()) {
            let recap = match heads.len() {
                0 => String::new(),
                _ => format!(
                    "To recap: {}.",
                    join_and(&heads.iter().map(|h| lower_first(h)).collect::<Vec<_>>())
                ),
            };
            self.scenes.push(Scene {
                id: "checklist".into(),
                kind: SceneKind::Segment,
                layout: Layout::Checklist,
                title: "Checklist".into(),
                subtitle: String::new(),
                heading: "Before you finish".into(),
                lines: vec![Line::draft(recap)],
                points: heads
                    .iter()
                    .map(|h| Point {
                        head: h.clone(),
                        sub: String::new(),
                        tone: Tone::Ok,
                        line: 0,
                    })
                    .collect(),
                image: None,
                highlights: Vec::new(),
                focus: None,
                source: steps.iter().map(|s| s.0).collect(),
                why: vec![format!("{} steps: a checklist recaps them", heads.len())],
                status: Default::default(),
            });
        }
    }

    fn faq(&mut self, sections: &[&Section]) {
        for s in sections {
            if s.name.trim_end().ends_with('?') {
                self.section_scenes(s);
                continue;
            }
            // questions inside a section: each bold or headed question opens a scene
            let mut q: Option<(usize, Vec<usize>)> = None;
            let mut out: Vec<(usize, Vec<usize>)> = Vec::new();
            for i in &s.blocks {
                let Some(b) = self.block(*i) else { continue };
                if (b.all_bold || b.lead.is_some() || b.kind == Kind::Heading)
                    && b.text.trim_end().ends_with('?')
                {
                    if let Some(done) = q.take() {
                        out.push(done);
                    }
                    q = Some((*i, Vec::new()));
                } else if let Some((_, rest)) = q.as_mut() {
                    rest.push(*i);
                }
            }
            out.extend(q);
            for (head, rest) in out {
                let name = self.block(head).map(|b| b.text.clone()).unwrap_or_default();
                let sec = Section {
                    name,
                    header: Some(head),
                    blocks: rest,
                    why: "a question".into(),
                };
                self.section_scenes(&sec);
            }
        }
    }

    fn reference(&mut self, sections: &[&Section]) {
        for s in sections {
            if s.header.is_some() {
                self.section_scenes(s);
            } else {
                let prose: Vec<usize> = s
                    .blocks
                    .iter()
                    .copied()
                    .filter(|i| self.block(*i).is_some_and(|b| b.kind != Kind::Image))
                    .collect();
                if !prose.is_empty() {
                    self.points_scene("Overview", &prose, None, &s.why);
                }
                let images: Vec<usize> = s
                    .blocks
                    .iter()
                    .copied()
                    .filter(|i| self.block(*i).is_some_and(|b| b.kind == Kind::Image))
                    .collect();
                if !images.is_empty() {
                    self.section_scenes(&Section {
                        name: "Overview".into(),
                        header: None,
                        blocks: s.blocks.clone(),
                        why: s.why.clone(),
                    });
                }
            }
        }
    }

    /// "Users with X" and "Users without X": a two-column scene says which group you are in.
    fn compare_if_parallel(&mut self, named: &[&Section]) {
        for pair in named.windows(2) {
            let (Some(a), Some(b)) = (pair.first(), pair.get(1)) else {
                continue;
            };
            let first_word = |s: &str| s.split_whitespace().next().unwrap_or("").to_lowercase();
            let negated = |s: &str| {
                let l = format!(" {} ", s.to_lowercase());
                [" without ", " not ", " no ", " don't ", " non-"]
                    .iter()
                    .any(|w| l.contains(w))
            };
            let has_image = |s: &Section| {
                s.blocks
                    .iter()
                    .any(|i| self.block(*i).is_some_and(|b| b.kind == Kind::Image))
            };
            if first_word(&a.name) == first_word(&b.name)
                && negated(&a.name) != negated(&b.name)
                && !has_image(a)
                && !has_image(b)
            {
                let left = condense(&self.say(&a.name), 10, 0).0;
                let right = condense(&self.say(&b.name), 10, 0).0;
                self.scenes.push(Scene {
                    id: "which".into(),
                    kind: SceneKind::Segment,
                    layout: Layout::Compare,
                    title: "Which one are you?".into(),
                    subtitle: String::new(),
                    heading: String::new(),
                    lines: vec![Line::draft(format!("{left}, or {}", lower_first(&right)))],
                    points: vec![
                        Point {
                            head: left,
                            sub: String::new(),
                            tone: Tone::Ok,
                            line: 0,
                        },
                        Point {
                            head: right,
                            sub: String::new(),
                            tone: Tone::Warn,
                            line: 0,
                        },
                    ],
                    image: None,
                    highlights: Vec::new(),
                    focus: None,
                    source: a.header.into_iter().chain(b.header).collect(),
                    why: vec!["two parallel sections, one the negation of the other".into()],
                    status: Default::default(),
                });
                return;
            }
        }
    }

    /// One scene per picture in the section (the text up to each picture goes with it), or one
    /// points scene when it has none.
    fn section_scenes(&mut self, s: &Section) {
        let mut chunks: Vec<(Vec<usize>, Option<String>)> = Vec::new();
        let mut cur: Vec<usize> = Vec::new();
        for i in &s.blocks {
            let Some(b) = self.block(*i) else { continue };
            if b.kind == Kind::Image {
                chunks.push((std::mem::take(&mut cur), b.image.clone()));
            } else {
                cur.push(*i);
            }
        }
        if !cur.is_empty() {
            match chunks.last_mut() {
                // a short tail after the last picture still belongs to it
                Some((text, _)) if cur.len() <= 1 => text.append(&mut cur),
                _ => chunks.push((cur, None)),
            }
        }
        if chunks.is_empty() {
            chunks.push((Vec::new(), None));
        }
        let header_title = self.title_from(&s.name);
        for (n, (blocks, image)) in chunks.into_iter().enumerate() {
            let header = if n == 0 { s.header } else { None };
            let sents = self.sents(&blocks, header);
            let lines = self.pick(&sents, 3);
            let title = if n == 0 {
                header_title.clone()
            } else {
                let actionable = sents.iter().find(|x| {
                    let l = self.say(&x.text).to_lowercase();
                    l.contains("you can") || imperative(&strip_conjunction(&l))
                });
                actionable
                    .or_else(|| sents.first())
                    .map(|x| self.title_from(&x.text))
                    .unwrap_or_else(|| format!("{header_title}, continued"))
            };
            let heading = if n == 0 {
                self.say(&s.name).trim_end_matches('.').to_string()
            } else {
                title.clone()
            };
            let mut points = self.callouts(&sents, &lines);
            // the heading already says it: no callout repeating it
            let norm = |x: &str| x.trim().trim_end_matches(['.', '!']).to_lowercase();
            points.retain(|pt| norm(&pt.head) != norm(&heading));
            let (layout, why) = match &image {
                Some(img) => (
                    Layout::Screenshot,
                    format!("{}; has a picture ({img})", s.why),
                ),
                None => (
                    Layout::Points,
                    format!("{}; no picture, so key points", s.why),
                ),
            };
            self.scenes.push(Scene {
                id: String::new(),
                kind: SceneKind::Segment,
                layout,
                title,
                subtitle: String::new(),
                heading,
                lines: lines.into_iter().map(|(t, _)| Line::draft(t)).collect(),
                points,
                image,
                highlights: Vec::new(),
                focus: None,
                source: header.into_iter().chain(blocks).collect(),
                why: vec![why],
                status: Default::default(),
            });
        }
    }

    fn points_scene(&mut self, title: &str, blocks: &[usize], header: Option<usize>, why: &str) {
        let sents = self.sents(blocks, header);
        if sents.is_empty() {
            return;
        }
        let lines = self.pick(&sents, 3);
        let points = self.callouts(&sents, &lines);
        self.scenes.push(Scene {
            id: String::new(),
            kind: SceneKind::Segment,
            layout: Layout::Points,
            title: title.into(),
            subtitle: String::new(),
            heading: title.into(),
            lines: lines.into_iter().map(|(t, _)| Line::draft(t)).collect(),
            points,
            image: None,
            highlights: Vec::new(),
            focus: None,
            source: header.into_iter().chain(blocks.iter().copied()).collect(),
            why: vec![format!("{why}; key points")],
            status: Default::default(),
        });
    }

    /// Callouts: one per narration line (so each appears as it is spoken), condensed, with a tone.
    fn callouts(&self, sents: &[Sent], lines: &[(String, usize)]) -> Vec<Point> {
        let mut out: Vec<Point> = Vec::new();
        for (n, (line, order)) in lines.iter().enumerate() {
            if out.last().is_some_and(|p: &Point| p.line == n) {
                continue;
            }
            let src = sents.get(*order).map(|s| s.header).unwrap_or(false);
            let (head, sub) = condense(&strip_conjunction(line), 9, 12);
            if src && n == 0 && lines.len() > 1 {
                // the header is already the page heading; its callout says it briefly
                out.push(Point {
                    head,
                    sub: String::new(),
                    tone: tone(line),
                    line: n,
                });
            } else {
                out.push(Point {
                    head,
                    sub,
                    tone: tone(line),
                    line: n,
                });
            }
        }
        out.truncate(4);
        out
    }

    /// A lower-third title: short enough that Showrig draws it whole.
    fn title_from(&self, s: &str) -> String {
        let spoken = self.say(s);
        let t = spoken.trim_end_matches('.');
        let t = strip_conjunction(t);
        if text::title_em(&t) <= text::LOWER_THIRD_EM {
            return t;
        }
        // "If you already use X or Y, you're all set" → the clause before the comma when it fits
        if let Some((a, _)) = t.split_once(", ")
            && text::title_em(a) <= text::LOWER_THIRD_EM
            && words(a) >= 3
        {
            return a.to_string();
        }
        text::fit_title(&t, text::LOWER_THIRD_EM)
    }

    fn outro(&mut self, t: DocType, all: &str) {
        let spoken_all = self.say(all);
        let target = call_to_action(&spoken_all);
        let domain = first_domain(all);
        let (title, line) = match (&target, t) {
            (Some(x), _) => (
                format!("Set up {x} today"),
                format!("Set up {x} today, and you're ready for the change."),
            ),
            (None, DocType::Procedure) => ("You're done".into(), "That's it: you're done.".into()),
            (None, _) => (
                "That's the change".into(),
                "That covers what you need to know.".into(),
            ),
        };
        self.scenes.push(Scene {
            id: "outro".into(),
            kind: SceneKind::Outro,
            layout: Layout::Title,
            title: text::fit_title(&title, text::centred_title_em(&title)),
            subtitle: domain.unwrap_or_default(),
            heading: String::new(),
            lines: vec![Line::draft(line)],
            points: Vec::new(),
            image: None,
            highlights: Vec::new(),
            focus: None,
            source: Vec::new(),
            why: vec![match &target {
                Some(x) => format!("the document asks readers to set up {x} most often"),
                None => "no single call to action found".into(),
            }],
            status: Default::default(),
        });
    }
}

/// The thing the document most often asks you to "set up": "Okta Verify".
fn call_to_action(text: &str) -> Option<String> {
    let re = regex::Regex::new(
        r"\bset ?up (?:a new |an? |the |your )?((?:[A-Z][\w-]*)(?: [A-Z][\w-]*){0,3})",
    )
    .ok()?;
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for c in re.captures_iter(text) {
        if let Some(m) = c.get(1) {
            *counts.entry(m.as_str().to_string()).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
        .map(|(k, _)| k)
}

fn first_domain(text: &str) -> Option<String> {
    let re = regex::Regex::new(
        r"\b(?:https?://)?((?:[a-zA-Z0-9-]+\.)+(?:com|org|net|edu|gov|io|app|dev))\b",
    )
    .ok()?;
    re.captures(text)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

/// "During this transition time from Nov 2-15, 2026, if you …" → "If you …".
fn strip_through_date(s: &str) -> String {
    let Some(d) = dates(s).into_iter().next() else {
        return s.to_string();
    };
    let Some(pos) = s.find(&d.text) else {
        return s.to_string();
    };
    let after = s
        .get(pos + d.text.len()..)
        .unwrap_or("")
        .trim_start_matches([',', ' ']);
    let after = after
        .trim_start_matches("onward")
        .trim_start_matches([',', ' ']);
    if words(after) < 3 {
        s.to_string()
    } else {
        text::capitalize(after)
    }
}

fn split_long(s: &str) -> Vec<String> {
    if words(s) <= 30 {
        return vec![s.to_string()];
    }
    for sep in ["; ", ", but ", " but ", ", so ", ", and "] {
        if let Some(pos) = s.find(sep) {
            let (a, b) = (
                s.get(..pos).unwrap_or(s),
                s.get(pos + sep.len()..).unwrap_or(""),
            );
            if words(a) >= 6 && words(b) >= 6 {
                let b = b.trim_start_matches("but ").trim_start_matches("so ");
                // "…, but will be notified" has lost its subject: give it back
                let b = match b.split_once(' ') {
                    Some(("will", rest)) => format!("you'll {rest}"),
                    Some(("can", rest)) => format!("you can {rest}"),
                    Some(("are", rest)) => format!("you're {rest}"),
                    Some(("need", rest)) => format!("you need {rest}"),
                    _ => b.to_string(),
                };
                return vec![
                    format!("{}.", a.trim_end_matches(',')),
                    text::capitalize(&b),
                ];
            }
        }
    }
    vec![s.to_string()]
}

fn lower_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        // keep acronyms and names as they are
        Some(f) if c.clone().next().is_some_and(char::is_lowercase) => {
            f.to_lowercase().collect::<String>() + c.as_str()
        }
        Some(f) => f.to_string() + c.as_str(),
        None => String::new(),
    }
}

fn join_and(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [a] => a.clone(),
        [a, b] => format!("{a} and {b}"),
        [rest @ .., last] => format!("{}, and {last}", rest.join(", ")),
    }
}

fn number_word(n: usize) -> String {
    ["Zero", "One", "Two", "Three", "Four", "Five", "Six"]
        .get(n)
        .map(|s| s.to_string())
        .unwrap_or_else(|| n.to_string())
}

fn unique_ids(scenes: &mut [Scene]) {
    let mut used: Vec<String> = Vec::new();
    for s in scenes.iter_mut() {
        let base = if s.id.is_empty() {
            text::slug(&s.title, 24)
        } else {
            s.id.clone()
        };
        let base = if base.is_empty() {
            "scene".to_string()
        } else {
            base
        };
        let mut id = base.clone();
        let mut n = 2;
        while used.contains(&id) {
            id = format!("{base}-{n}");
            n += 1;
        }
        used.push(id.clone());
        s.id = id;
    }
}

/// "But from account settings…" → "From account settings…".
fn strip_conjunction(s: &str) -> String {
    for c in [
        "But ",
        "And ",
        "So ",
        "Also, ",
        "However, ",
        "but ",
        "and ",
        "so ",
    ] {
        if let Some(rest) = s.strip_prefix(c) {
            return text::capitalize(rest);
        }
    }
    s.to_string()
}
