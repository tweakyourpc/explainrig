//! Markdown reader (CommonMark plus tables): headings, paragraphs, list items with their level,
//! bold lead-ins, images (loaded from disk, relative to the file) and tables.

use std::path::{Path, PathBuf};

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::{Block, Document, Error, Image, Kind, image_ext, image_size, tidy};

const MAX_IMAGE: u64 = 32 * 1024 * 1024;

pub fn read_markdown(path: &Path) -> Result<Document, Error> {
    let text = std::fs::read_to_string(path)?;
    let base = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut doc = read_markdown_str(&text, Some(&base));
    doc.source = path.display().to_string();
    Ok(doc)
}

/// Parse Markdown text. Images are loaded relative to `base` when it is given; remote or missing
/// images are skipped.
pub fn read_markdown_str(text: &str, base: Option<&Path>) -> Document {
    let mut doc = Document::default();
    let mut cur: Option<Block> = None;
    let mut runs: Vec<(String, bool)> = Vec::new();
    let mut strong = 0usize;
    let mut lists: Vec<bool> = Vec::new(); // ordered? per nesting level
    let mut list_ids: Vec<String> = Vec::new();
    let mut list_count = 0usize;
    let mut table: Option<Vec<Vec<String>>> = None;
    let mut in_image: Option<(String, String)> = None; // (dest, alt so far)
    let mut code: Option<String> = None;

    let finish = |cur: &mut Option<Block>, runs: &mut Vec<(String, bool)>, doc: &mut Document| {
        if let Some(mut b) = cur.take() {
            let text = tidy(&runs.iter().map(|(t, _)| t.as_str()).collect::<String>());
            let words: Vec<&(String, bool)> =
                runs.iter().filter(|(t, _)| !t.trim().is_empty()).collect();
            b.all_bold = !words.is_empty() && words.iter().all(|(_, s)| *s);
            let lead = tidy(
                &runs
                    .iter()
                    .take_while(|(t, s)| *s || t.trim().is_empty())
                    .map(|(t, _)| t.as_str())
                    .collect::<String>(),
            );
            if !lead.is_empty() {
                b.lead = Some(lead);
            }
            b.text = text;
            if !b.text.is_empty() {
                doc.blocks.push(b);
            }
        }
        runs.clear();
    };

    for ev in Parser::new_ext(text, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH) {
        if let Some((_, alt)) = in_image.as_mut() {
            match ev {
                Event::Text(t) | Event::Code(t) => alt.push_str(&t),
                Event::End(TagEnd::Image) => {
                    if let Some((dest, alt)) = in_image.take() {
                        // the paragraph text so far stands on its own, then the picture
                        let kind_level = cur.as_ref().map(|b| (b.kind, b.level, b.ordered));
                        finish(&mut cur, &mut runs, &mut doc);
                        add_image(&mut doc, base, &dest, &alt);
                        if let Some((kind, level, ordered)) = kind_level {
                            let mut b = Block::new(kind, "");
                            b.level = level;
                            b.ordered = ordered;
                            cur = Some(b);
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        if let Some(c) = code.as_mut() {
            match ev {
                Event::Text(t) => c.push_str(&t),
                Event::End(TagEnd::CodeBlock) => {
                    let body = code.take().unwrap_or_default();
                    let body = body.trim_end().to_string();
                    if !body.is_empty() {
                        doc.blocks.push(Block::new(Kind::Code, body));
                    }
                }
                _ => {}
            }
            continue;
        }
        if let Some(t) = table.as_mut() {
            match ev {
                Event::Start(Tag::TableHead | Tag::TableRow) => t.push(Vec::new()),
                Event::Start(Tag::TableCell) => {
                    if let Some(row) = t.last_mut() {
                        row.push(String::new());
                    }
                }
                Event::Text(s) | Event::Code(s) => {
                    if let Some(cell) = t.last_mut().and_then(|r| r.last_mut()) {
                        cell.push_str(&s);
                    }
                }
                Event::End(TagEnd::Table) => {
                    let rows: Vec<Vec<String>> = table
                        .take()
                        .unwrap_or_default()
                        .into_iter()
                        .map(|r| r.iter().map(|c| tidy(c)).collect())
                        .collect();
                    let mut b = Block::new(
                        Kind::Table,
                        rows.iter()
                            .flatten()
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(" · "),
                    );
                    b.rows = rows;
                    doc.blocks.push(b);
                }
                _ => {}
            }
            continue;
        }
        match ev {
            Event::Start(Tag::Heading { level, .. }) => {
                finish(&mut cur, &mut runs, &mut doc);
                let mut b = Block::new(Kind::Heading, "");
                b.level = match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    HeadingLevel::H3 => 3,
                    HeadingLevel::H4 => 4,
                    HeadingLevel::H5 => 5,
                    HeadingLevel::H6 => 6,
                };
                cur = Some(b);
            }
            Event::Start(Tag::Paragraph) if cur.is_none() => {
                cur = Some(Block::new(Kind::Paragraph, ""));
            }
            Event::Start(Tag::List(first)) => {
                finish(&mut cur, &mut runs, &mut doc);
                lists.push(first.is_some());
                list_count += 1;
                list_ids.push(format!("md{list_count}"));
            }
            Event::End(TagEnd::List(_)) => {
                finish(&mut cur, &mut runs, &mut doc);
                lists.pop();
                list_ids.pop();
            }
            Event::Start(Tag::Item) => {
                finish(&mut cur, &mut runs, &mut doc);
                let mut b = Block::new(Kind::Item, "");
                b.level = u8::try_from(lists.len().saturating_sub(1)).unwrap_or(u8::MAX);
                b.ordered = lists.last().copied().unwrap_or(false);
                b.list = list_ids.last().cloned();
                cur = Some(b);
            }
            Event::End(
                TagEnd::Heading(_) | TagEnd::Paragraph | TagEnd::Item | TagEnd::BlockQuote(_),
            ) => {
                finish(&mut cur, &mut runs, &mut doc);
            }
            Event::Start(Tag::Strong) => strong += 1,
            Event::End(TagEnd::Strong) => strong = strong.saturating_sub(1),
            Event::Start(Tag::Image { dest_url, .. }) => {
                in_image = Some((dest_url.to_string(), String::new()))
            }
            Event::Start(Tag::CodeBlock(_)) => {
                finish(&mut cur, &mut runs, &mut doc);
                code = Some(String::new());
            }
            Event::Start(Tag::Table(_)) => {
                finish(&mut cur, &mut runs, &mut doc);
                table = Some(Vec::new());
            }
            Event::Text(t) | Event::Code(t) => {
                if cur.is_none() {
                    cur = Some(Block::new(Kind::Paragraph, ""));
                }
                let bold = strong > 0;
                match runs.last_mut() {
                    Some((s, b)) if *b == bold => s.push_str(&t),
                    _ => runs.push((t.to_string(), bold)),
                }
            }
            Event::SoftBreak | Event::HardBreak => runs.push((" ".into(), strong > 0)),
            _ => {}
        }
    }
    finish(&mut cur, &mut runs, &mut doc);
    doc
}

fn add_image(doc: &mut Document, base: Option<&Path>, dest: &str, alt: &str) {
    if dest.contains("://") {
        return;
    }
    let Some(base) = base else { return };
    let path: PathBuf = base.join(dest.split(['?', '#']).next().unwrap_or(dest));
    let Ok(meta) = std::fs::metadata(&path) else {
        return;
    };
    if meta.len() > MAX_IMAGE {
        return;
    }
    let Ok(data) = std::fs::read(&path) else {
        return;
    };
    let id = format!("image{}", doc.images.len() + 1);
    let (width, height) = image_size(&data);
    doc.images.push(Image {
        id: id.clone(),
        ext: image_ext(dest),
        width,
        height,
        alt: tidy(alt),
        data,
    });
    let mut b = Block::new(Kind::Image, tidy(alt));
    b.image = Some(id);
    doc.blocks.push(b);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structure() {
        let md = "# Title\n\nIntro **para**.\n\n## Steps\n\n1. **Open** the app\n2. Click *Set up*\n   - nested note\n\n| A | B |\n|---|---|\n| 1 | 2 |\n";
        let d = read_markdown_str(md, None);
        let kinds: Vec<(Kind, u8)> = d.blocks.iter().map(|b| (b.kind, b.level)).collect();
        assert_eq!(
            kinds,
            vec![
                (Kind::Heading, 1),
                (Kind::Paragraph, 0),
                (Kind::Heading, 2),
                (Kind::Item, 0),
                (Kind::Item, 0),
                (Kind::Item, 1),
                (Kind::Table, 0)
            ]
        );
        let open = d
            .blocks
            .get(3)
            .map(|b| (b.ordered, b.lead.clone(), b.text.clone()));
        assert_eq!(
            open,
            Some((true, Some("Open".into()), "Open the app".into()))
        );
        assert_eq!(d.blocks.get(5).map(|b| b.ordered), Some(false));
        assert_eq!(d.blocks.get(6).map(|b| b.rows.len()), Some(2));
    }
}
