//! Word `.docx` (WordprocessingML) reader: paragraphs with their style, list level and numbering,
//! bold lead-ins, pictures and tables, in reading order.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use crate::{Block, Document, Error, Image, Kind, image_ext, image_size, tidy};

/// Largest part we read out of the zip (document.xml of a long manual is a few MB).
const MAX_PART: u64 = 64 * 1024 * 1024;

pub fn read_docx(path: &Path) -> Result<Document, Error> {
    let file = std::fs::File::open(path)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| Error::Docx(e.to_string()))?;
    let mut part = |name: &str| -> Result<Option<Vec<u8>>, Error> {
        let Ok(entry) = zip.by_name(name) else {
            return Ok(None);
        };
        let mut buf = Vec::new();
        entry.take(MAX_PART).read_to_end(&mut buf)?;
        Ok(Some(buf))
    };
    let body = part("word/document.xml")?
        .ok_or_else(|| Error::Docx("word/document.xml is missing".into()))?;
    let rels = part("word/_rels/document.xml.rels")?
        .map(|b| relationships(&b))
        .unwrap_or_default();
    let styles = part("word/styles.xml")?
        .map(|b| styles(&b))
        .unwrap_or_default();
    let numbering = part("word/numbering.xml")?
        .map(|b| numbering(&b))
        .unwrap_or_default();

    let mut doc = Document {
        source: path.display().to_string(),
        ..Default::default()
    };
    let mut media: HashMap<String, String> = HashMap::new(); // zip path -> image id
    for p in paragraphs(&body) {
        match p {
            Raw::Table(rows) => {
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
            Raw::Para(p) => {
                let style = p.style.as_deref().and_then(|id| styles.get(id));
                let text = tidy(&p.runs.iter().map(|(t, _)| t.as_str()).collect::<String>());
                if !text.is_empty() {
                    doc.blocks.push(block(&p, style, &numbering, text));
                }
                for (rid, alt) in &p.images {
                    let Some(target) = rels.get(rid) else {
                        continue;
                    };
                    let zip_path = resolve(target);
                    let id = match media.get(&zip_path) {
                        Some(id) => id.clone(),
                        None => {
                            let Some(data) = part(&zip_path)? else {
                                continue;
                            };
                            let id = format!("image{}", doc.images.len() + 1);
                            let (width, height) = image_size(&data);
                            doc.images.push(Image {
                                id: id.clone(),
                                ext: image_ext(&zip_path),
                                width,
                                height,
                                alt: alt.clone(),
                                data,
                            });
                            media.insert(zip_path, id.clone());
                            id
                        }
                    };
                    let mut b = Block::new(Kind::Image, alt.clone());
                    b.image = Some(id);
                    doc.blocks.push(b);
                }
            }
        }
    }
    Ok(doc)
}

fn block(p: &Para, style: Option<&Style>, numbering: &Numbering, text: String) -> Block {
    let num = p
        .num_id
        .clone()
        .or_else(|| style.and_then(|s| s.num_id.clone()))
        .filter(|n| n != "0");
    let ilvl = p.ilvl.or_else(|| style.and_then(|s| s.ilvl)).unwrap_or(0);
    let mut b = if let Some(level) = style.and_then(|s| s.heading) {
        let mut b = Block::new(Kind::Heading, text);
        b.level = level;
        b
    } else if let Some(num) = num {
        let mut b = Block::new(Kind::Item, text);
        b.level = ilvl;
        b.ordered = numbering.ordered(&num, ilvl);
        b.list = Some(num);
        b
    } else {
        Block::new(Kind::Paragraph, text)
    };
    b.style = style.map(|s| s.name.clone());
    let words: Vec<&(String, bool)> = p
        .runs
        .iter()
        .filter(|(t, _)| !t.trim().is_empty())
        .collect();
    b.all_bold = !words.is_empty() && words.iter().all(|(_, bold)| *bold);
    let lead: String = p
        .runs
        .iter()
        .take_while(|(t, bold)| *bold || t.trim().is_empty())
        .map(|(t, _)| t.as_str())
        .collect();
    let lead = tidy(&lead);
    if !lead.is_empty() {
        b.lead = Some(lead);
    }
    b
}

/// `media/image1.png` relative to `word/`, or an absolute `/word/media/…` target.
fn resolve(target: &str) -> String {
    match target.strip_prefix('/') {
        Some(abs) => abs.to_string(),
        None => {
            let mut parts: Vec<&str> = vec!["word"];
            for seg in target.split('/') {
                match seg {
                    ".." => {
                        parts.pop();
                    }
                    "." | "" => {}
                    s => parts.push(s),
                }
            }
            parts.join("/")
        }
    }
}

fn attr(e: &BytesStart<'_>, key: &str) -> Option<String> {
    let a = e.try_get_attribute(key).ok().flatten()?;
    let raw = std::str::from_utf8(&a.value).ok()?;
    Some(
        quick_xml::escape::unescape(raw)
            .map(|v| v.into_owned())
            .unwrap_or_else(|_| raw.to_string()),
    )
}

fn local(e: &BytesStart<'_>) -> Vec<u8> {
    e.local_name().as_ref().to_vec()
}

fn relationships(xml: &[u8]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let text = String::from_utf8_lossy(xml);
    let mut r = Reader::from_str(&text);
    loop {
        match r.read_event() {
            Ok(Event::Start(e) | Event::Empty(e)) if local(&e) == b"Relationship" => {
                if let (Some(id), Some(target)) = (attr(&e, "Id"), attr(&e, "Target")) {
                    out.insert(id, target);
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

#[derive(Debug, Default, Clone)]
struct Style {
    name: String,
    heading: Option<u8>,
    num_id: Option<String>,
    ilvl: Option<u8>,
}

fn styles(xml: &[u8]) -> HashMap<String, Style> {
    let mut out = HashMap::new();
    let text = String::from_utf8_lossy(xml);
    let mut r = Reader::from_str(&text);
    let mut cur: Option<(String, Style)> = None;
    let mut outline: Option<u8> = None;
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) if local(&e) == b"style" => {
                cur = attr(&e, "w:styleId").map(|id| (id, Style::default()));
                outline = None;
            }
            Ok(Event::Start(e) | Event::Empty(e)) => {
                let Some((_, s)) = cur.as_mut() else { continue };
                match local(&e).as_slice() {
                    b"name" => s.name = attr(&e, "w:val").unwrap_or_default(),
                    b"outlineLvl" => outline = attr(&e, "w:val").and_then(|v| v.parse::<u8>().ok()),
                    b"numId" => s.num_id = attr(&e, "w:val"),
                    b"ilvl" => s.ilvl = attr(&e, "w:val").and_then(|v| v.parse().ok()),
                    _ => {}
                }
            }
            Ok(Event::End(e)) if e.local_name().as_ref() == b"style" => {
                if let Some((id, mut s)) = cur.take() {
                    let name = s.name.to_ascii_lowercase();
                    s.heading = if name == "title" {
                        Some(0)
                    } else if let Some(n) = name.strip_prefix("heading ") {
                        n.trim().parse::<u8>().ok()
                    } else {
                        outline.filter(|o| *o < 9).map(|o| o.saturating_add(1))
                    };
                    out.insert(id, s);
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

/// Which list levels are numbered (as opposed to bulleted).
#[derive(Debug, Default)]
struct Numbering {
    num_to_abstract: HashMap<String, String>,
    formats: HashMap<(String, u8), String>,
}

impl Numbering {
    fn ordered(&self, num_id: &str, ilvl: u8) -> bool {
        let Some(abs) = self.num_to_abstract.get(num_id) else {
            return false;
        };
        match self.formats.get(&(abs.clone(), ilvl)) {
            Some(fmt) => !matches!(fmt.as_str(), "bullet" | "none" | ""),
            None => false,
        }
    }
}

fn numbering(xml: &[u8]) -> Numbering {
    let mut out = Numbering::default();
    let text = String::from_utf8_lossy(xml);
    let mut r = Reader::from_str(&text);
    let mut abs: Option<String> = None;
    let mut lvl: Option<u8> = None;
    let mut num: Option<String> = None;
    loop {
        match r.read_event() {
            Ok(Event::Start(e) | Event::Empty(e)) => match local(&e).as_slice() {
                b"abstractNum" => abs = attr(&e, "w:abstractNumId"),
                b"lvl" => lvl = attr(&e, "w:ilvl").and_then(|v| v.parse().ok()),
                b"numFmt" => {
                    if let (Some(a), Some(l), Some(f)) = (abs.clone(), lvl, attr(&e, "w:val")) {
                        out.formats.entry((a, l)).or_insert(f);
                    }
                }
                b"num" => num = attr(&e, "w:numId"),
                b"abstractNumId" => {
                    if let (Some(n), Some(a)) = (num.clone(), attr(&e, "w:val")) {
                        out.num_to_abstract.insert(n, a);
                    }
                }
                _ => {}
            },
            Ok(Event::End(e)) => match e.local_name().as_ref() {
                b"abstractNum" => abs = None,
                b"lvl" => lvl = None,
                b"num" => num = None,
                _ => {}
            },
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

#[derive(Debug, Default)]
struct Para {
    style: Option<String>,
    num_id: Option<String>,
    ilvl: Option<u8>,
    /// (text, bold) per run.
    runs: Vec<(String, bool)>,
    /// (relationship id, alt text) per picture.
    images: Vec<(String, String)>,
}

enum Raw {
    Para(Para),
    Table(Vec<Vec<String>>),
}

/// Walk the body: paragraphs outside tables become [`Raw::Para`], each outermost table one [`Raw::Table`].
fn paragraphs(xml: &[u8]) -> Vec<Raw> {
    let text = String::from_utf8_lossy(xml);
    let mut r = Reader::from_str(&text);
    let mut out = Vec::new();
    let mut p: Option<Para> = None;
    let (mut in_ppr, mut in_rpr, mut in_t) = (false, false, false);
    let mut run_bold = false;
    let mut pending_alt = String::new();
    let mut tables = 0usize;
    let mut rows: Vec<Vec<String>> = Vec::new();

    let push_text = |p: &mut Option<Para>, s: &str, bold: bool| {
        if let Some(p) = p.as_mut() {
            match p.runs.last_mut() {
                Some((t, b)) if *b == bold => t.push_str(s),
                _ => p.runs.push((s.to_string(), bold)),
            }
        }
    };

    while let Ok(ev) = r.read_event() {
        match ev {
            Event::Start(e) => {
                let name = local(&e);
                match name.as_slice() {
                    b"tbl" => {
                        tables += 1;
                        if tables == 1 {
                            rows.clear();
                        }
                    }
                    b"tr" if tables == 1 => rows.push(Vec::new()),
                    b"tc" if tables == 1 => {
                        if let Some(row) = rows.last_mut() {
                            row.push(String::new());
                        }
                    }
                    b"p" => {
                        p = Some(Para::default());
                        run_bold = false;
                    }
                    b"pPr" => in_ppr = true,
                    b"rPr" => in_rpr = true,
                    b"r" => run_bold = false,
                    b"t" => in_t = true,
                    _ => on_empty(&e, &mut p, in_ppr, in_rpr, &mut run_bold, &mut pending_alt),
                }
            }
            Event::Empty(e) => {
                let name = local(&e);
                match name.as_slice() {
                    b"tab" | b"br" | b"cr" if !in_ppr => push_text(&mut p, " ", run_bold),
                    _ => on_empty(&e, &mut p, in_ppr, in_rpr, &mut run_bold, &mut pending_alt),
                }
            }
            Event::Text(t) if in_t => {
                if let Ok(s) = t.decode() {
                    push_text(&mut p, &s, run_bold);
                }
            }
            Event::GeneralRef(g) if in_t => {
                let s = match g.resolve_char_ref() {
                    Ok(Some(c)) => c.to_string(),
                    _ => match g.decode().as_deref() {
                        Ok("amp") => "&".into(),
                        Ok("lt") => "<".into(),
                        Ok("gt") => ">".into(),
                        Ok("quot") => "\"".into(),
                        Ok("apos") => "'".into(),
                        _ => String::new(),
                    },
                };
                push_text(&mut p, &s, run_bold);
            }
            Event::End(e) => match e.local_name().as_ref() {
                b"tbl" => {
                    tables = tables.saturating_sub(1);
                    if tables == 0 {
                        let cells: Vec<Vec<String>> = rows
                            .drain(..)
                            .map(|row| row.into_iter().map(|c| tidy(&c)).collect::<Vec<_>>())
                            .filter(|row: &Vec<String>| row.iter().any(|c| !c.is_empty()))
                            .collect();
                        if !cells.is_empty() {
                            out.push(Raw::Table(cells));
                        }
                    }
                }
                b"p" => {
                    if let Some(done) = p.take() {
                        if tables > 0 {
                            let t: String = done.runs.iter().map(|(t, _)| t.as_str()).collect();
                            if let Some(cell) = rows.last_mut().and_then(|r| r.last_mut()) {
                                cell.push(' ');
                                cell.push_str(&t);
                            }
                        } else {
                            out.push(Raw::Para(done));
                        }
                    }
                }
                b"pPr" => in_ppr = false,
                b"rPr" => in_rpr = false,
                b"t" => in_t = false,
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    out
}

/// Properties and pictures, whether they arrive as empty or start tags.
fn on_empty(
    e: &BytesStart<'_>,
    p: &mut Option<Para>,
    in_ppr: bool,
    in_rpr: bool,
    bold: &mut bool,
    alt: &mut String,
) {
    let name = local(e);
    match name.as_slice() {
        b"pStyle" if in_ppr => {
            if let Some(p) = p.as_mut() {
                p.style = attr(e, "w:val");
            }
        }
        b"numId" if in_ppr => {
            if let Some(p) = p.as_mut() {
                p.num_id = attr(e, "w:val");
            }
        }
        b"ilvl" if in_ppr => {
            if let Some(p) = p.as_mut() {
                p.ilvl = attr(e, "w:val").and_then(|v| v.parse().ok());
            }
        }
        b"b" if in_rpr && !in_ppr => {
            *bold = !matches!(attr(e, "w:val").as_deref(), Some("0" | "false" | "off"));
        }
        b"rStyle"
            if in_rpr
                && !in_ppr
                && attr(e, "w:val").is_some_and(|v| v.eq_ignore_ascii_case("strong")) =>
        {
            *bold = true;
        }
        b"docPr" => {
            *alt = attr(e, "descr")
                .or_else(|| attr(e, "title"))
                .unwrap_or_default()
        }
        b"blip" | b"imagedata" => {
            let rid = attr(e, "r:embed").or_else(|| attr(e, "r:id"));
            if let (Some(p), Some(rid)) = (p.as_mut(), rid) {
                p.images.push((rid, std::mem::take(alt)));
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_targets() {
        assert_eq!(resolve("media/image1.png"), "word/media/image1.png");
        assert_eq!(resolve("../media/x.png"), "media/x.png");
        assert_eq!(resolve("/word/media/a.jpeg"), "word/media/a.jpeg");
    }
}
