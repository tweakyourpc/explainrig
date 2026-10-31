//! The Explainrig project (`explainrig.json`): the guessed storyboard and everything the person
//! changed or approved. Serialized deterministically (ordered maps, no floats from clocks).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const SCHEMA: u32 = 1;

/// What kind of document the rules think this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DocType {
    /// Something changes on a date: retirements, migrations, new requirements.
    ChangeNotice,
    /// Numbered steps to follow.
    Procedure,
    /// Questions and answers.
    Faq,
    /// Everything else: sections of facts.
    Reference,
}

impl DocType {
    pub const ALL: [DocType; 4] = [
        DocType::ChangeNotice,
        DocType::Procedure,
        DocType::Faq,
        DocType::Reference,
    ];

    pub fn label(self) -> &'static str {
        match self {
            DocType::ChangeNotice => "change notice",
            DocType::Procedure => "procedure",
            DocType::Faq => "FAQ",
            DocType::Reference => "reference",
        }
    }
}

/// How a scene looks. `Title` is a Showrig title card; the others are Explainrig pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Layout {
    Title,
    Timeline,
    Screenshot,
    Steps,
    Compare,
    Points,
    Checklist,
}

impl Layout {
    pub const ALL: [Layout; 7] = [
        Layout::Title,
        Layout::Timeline,
        Layout::Screenshot,
        Layout::Steps,
        Layout::Compare,
        Layout::Points,
        Layout::Checklist,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Layout::Title => "Title card",
            Layout::Timeline => "Timeline",
            Layout::Screenshot => "Screenshot + callouts",
            Layout::Steps => "Numbered steps",
            Layout::Compare => "Two columns",
            Layout::Points => "Key points",
            Layout::Checklist => "Checklist",
        }
    }
}

/// The colour and icon of a callout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Tone {
    Ok,
    Warn,
    Bad,
    #[default]
    Info,
}

/// One callout, timeline stop, step or column entry on a page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Point {
    /// Bold first line ("November 2–15", "You're all set").
    pub head: String,
    /// Smaller second line; may be empty.
    #[serde(default)]
    pub sub: String,
    #[serde(default)]
    pub tone: Tone,
    /// The narration line (0-based) this point appears with.
    #[serde(default)]
    pub line: usize,
}

/// A spoken line. `draft` lines were written by the rules, not by a person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Line {
    pub text: String,
    #[serde(default)]
    pub draft: bool,
}

impl Line {
    pub fn draft(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            draft: true,
        }
    }
}

/// A box drawn on a screenshot, in the image's own pixels, shown from a narration line onwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Highlight {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    #[serde(default)]
    pub line: usize,
    /// Move the cursor to the box and click it.
    #[serde(default)]
    pub click: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    #[default]
    Guess,
    Edited,
    Approved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SceneKind {
    Intro,
    Segment,
    Card,
    Outro,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scene {
    pub id: String,
    pub kind: SceneKind,
    pub layout: Layout,
    pub title: String,
    #[serde(default)]
    pub subtitle: String,
    /// The heading shown on the page itself (pages only).
    #[serde(default)]
    pub heading: String,
    #[serde(default)]
    pub lines: Vec<Line>,
    #[serde(default)]
    pub points: Vec<Point>,
    /// An [`ImageRef::id`] for screenshot scenes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(default)]
    pub highlights: Vec<Highlight>,
    /// The point this scene is about, for step lists that show every step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<usize>,
    /// The block indices of the source document this scene was made from.
    #[serde(default)]
    pub source: Vec<usize>,
    /// Why the rules made this scene the way they did.
    #[serde(default)]
    pub why: Vec<String>,
    #[serde(default)]
    pub status: Status,
}

/// A section the rules left out, kept so the person can bring it back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dropped {
    pub name: String,
    pub text: String,
    pub why: String,
    #[serde(default)]
    pub source: Vec<usize>,
}

/// A picture copied out of the document into the project's `img/` folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageRef {
    pub id: String,
    /// Relative to the project folder: `img/image1.png`.
    pub file: String,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub alt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Style {
    pub accent: String,
    pub background: String,
    pub voice: String,
    pub music: String,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            accent: "#f39200".into(),
            background: "#14213d".into(),
            voice: "af_heart".into(),
            music: "calm".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub path: String,
    /// FNV-1a 64 of the file's bytes, hex: tells a re-import whether the document changed.
    pub hash: String,
}

/// How strongly each document type matched, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Score {
    pub doc_type: DocType,
    pub score: i32,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub explainrig: u32,
    pub title: String,
    pub source: Source,
    pub doc_type: DocType,
    pub scores: Vec<Score>,
    pub style: Style,
    pub scenes: Vec<Scene>,
    #[serde(default)]
    pub dropped: Vec<Dropped>,
    #[serde(default)]
    pub images: Vec<ImageRef>,
    /// Pronunciations the person set; they win over the automatic ones. "" removes an automatic one.
    #[serde(default)]
    pub say: BTreeMap<String, String>,
}

impl Project {
    pub fn image(&self, id: &str) -> Option<&ImageRef> {
        self.images.iter().find(|i| i.id == id)
    }

    /// True when every scene is approved, which is what a final render requires.
    pub fn approved(&self) -> bool {
        !self.scenes.is_empty() && self.scenes.iter().all(|s| s.status == Status::Approved)
    }

    /// A scene id not used yet, from a title.
    pub fn fresh_id(&self, title: &str) -> String {
        let base = crate::text::slug(title, 24);
        let base = if base.is_empty() {
            "scene".to_string()
        } else {
            base
        };
        let mut id = base.clone();
        let mut n = 2;
        while self.scenes.iter().any(|s| s.id == id) {
            id = format!("{base}-{n}");
            n += 1;
        }
        id
    }
}

/// FNV-1a 64, hex.
pub fn fingerprint(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}
