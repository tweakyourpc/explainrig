//! Explainrig ingest: a Word `.docx` or a Markdown file becomes one ordered list of [`Block`]s.
//!
//! The block list keeps only what the rules in `explainrig-guess` look at: what kind of block it is,
//! its heading or list level, its text, the bold run it starts with (a "lead-in"), and the images and
//! tables in the order they appear. Nothing here interprets the document.

mod docx;
mod markdown;

use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};

pub use docx::read_docx;
pub use markdown::{read_markdown, read_markdown_str};

/// What a block is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A heading; `level` is 1 for the top level. A Word "Title" is level 0.
    Heading,
    Paragraph,
    /// A list item; `level` is 0 for the outer list, `ordered` is true for numbered lists.
    Item,
    /// A picture; `image` names it in [`Document::images`].
    Image,
    /// A table; `rows` holds the cell text.
    Table,
    /// Preformatted text (Markdown code blocks).
    Code,
}

/// One unit of the document, in reading order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Block {
    pub kind: Kind,
    #[serde(default)]
    pub level: u8,
    #[serde(default)]
    pub ordered: bool,
    #[serde(default)]
    pub text: String,
    /// Which list an item belongs to (the Word numbering id, or `md<n>` per Markdown list).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list: Option<String>,
    /// The bold run the block starts with, when it has one ("Background:", "Users with Okta Verify").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lead: Option<String>,
    /// True when every word of the text is bold or strong.
    #[serde(default)]
    pub all_bold: bool,
    /// The [`Image::id`] of a picture block.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<Vec<String>>,
    /// The Word paragraph style name, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
}

impl Block {
    pub fn new(kind: Kind, text: impl Into<String>) -> Self {
        Self {
            kind,
            level: 0,
            ordered: false,
            text: text.into(),
            list: None,
            lead: None,
            all_bold: false,
            image: None,
            rows: Vec::new(),
            style: None,
        }
    }
}

/// A picture taken from the document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Image {
    /// Stable within the document: `image1`, `image2`, … in reading order.
    pub id: String,
    /// The file extension, lower case (`png`, `jpeg`, …).
    pub ext: String,
    pub width: u32,
    pub height: u32,
    /// Alt text or description, when the document gives one.
    #[serde(default)]
    pub alt: String,
    #[serde(skip)]
    pub data: Vec<u8>,
}

/// A whole document.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub source: String,
    pub blocks: Vec<Block>,
    pub images: Vec<Image>,
}

impl Document {
    pub fn image(&self, id: &str) -> Option<&Image> {
        self.images.iter().find(|i| i.id == id)
    }
}

/// Why a document could not be read.
#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    /// The file is not a zip, or a part inside it is missing or malformed.
    Docx(String),
    Unsupported(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::Docx(e) => write!(f, "not a readable .docx: {e}"),
            Error::Unsupported(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

/// Read a `.docx`, `.md` or `.markdown` file, choosing the reader by extension.
pub fn read(path: &Path) -> Result<Document, Error> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "docx" => read_docx(path),
        "md" | "markdown" | "txt" => read_markdown(path),
        _ => Err(Error::Unsupported(format!(
            "{}: only .docx and Markdown (.md) are supported",
            path.display()
        ))),
    }
}

/// Collapse runs of whitespace into single spaces and trim the ends.
pub(crate) fn tidy(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Pixel size of a PNG or JPEG, read from its header; (0, 0) when unknown.
pub(crate) fn image_size(data: &[u8]) -> (u32, u32) {
    let be32 = |b: &[u8]| -> Option<u32> { Some(u32::from_be_bytes(b.get(..4)?.try_into().ok()?)) };
    if data.starts_with(b"\x89PNG\r\n\x1a\n")
        && let (Some(w), Some(h)) = (
            data.get(16..20).and_then(be32),
            data.get(20..24).and_then(be32),
        )
    {
        return (w, h);
    }
    if data.starts_with(&[0xFF, 0xD8]) {
        let mut i = 2usize;
        while let (Some(&m), Some(&kind)) = (data.get(i), data.get(i + 1)) {
            if m != 0xFF {
                break;
            }
            let len = match data.get(i + 2..i + 4) {
                Some(&[a, b]) => Some(u16::from_be_bytes([a, b]) as usize),
                _ => None,
            };
            let Some(len) = len else { break };
            if matches!(kind, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF) {
                if let Some(&[h0, h1, w0, w1]) = data.get(i + 5..i + 9) {
                    return (
                        u16::from_be_bytes([w0, w1]) as u32,
                        u16::from_be_bytes([h0, h1]) as u32,
                    );
                }
                break;
            }
            i = i.saturating_add(2).saturating_add(len);
        }
    }
    (0, 0)
}

/// The extension for an image file name or MIME-ish path, lower case, `jpeg` for `jpg`.
pub(crate) fn image_ext(name: &str) -> String {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    if ext == "jpg" { "jpeg".into() } else { ext }
}
