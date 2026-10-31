//! Text rules: sentences, the second-person rewrites, dates, tones, slugs. All deterministic.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;

use crate::model::Tone;

fn re(p: &str) -> Option<Regex> {
    Regex::new(p).ok()
}

const ABBREVIATIONS: &[&str] = &[
    "e.g", "i.e", "etc", "vs", "mr", "mrs", "ms", "dr", "st", "no", "approx", "dept", "jan", "feb",
    "mar", "apr", "jun", "jul", "aug", "sep", "sept", "oct", "nov", "dec", "inc", "ltd", "fig",
];

/// Split text into sentences at `.`, `!` or `?` followed by a space and a capital, digit or quote,
/// except after common abbreviations and single letters.
pub fn sentences(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars.get(i).copied().unwrap_or(' ');
        if matches!(c, '.' | '!' | '?') {
            let next = chars.get(i + 1).copied();
            let after = chars.get(i + 2).copied();
            let boundary = next.is_some_and(char::is_whitespace)
                && after.is_some_and(|a| {
                    a.is_uppercase() || a.is_ascii_digit() || matches!(a, '"' | '\'' | '(' | '*')
                });
            let end_of_text = next.is_none();
            if boundary || end_of_text {
                let word: String = chars
                    .get(start..i)
                    .unwrap_or(&[])
                    .iter()
                    .rev()
                    .take_while(|ch| !ch.is_whitespace())
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                let w = word.trim_start_matches(['(', '"']).to_ascii_lowercase();
                let abbreviation = c == '.'
                    && !end_of_text
                    && (ABBREVIATIONS.contains(&w.as_str()) || w.chars().count() == 1);
                if !abbreviation {
                    let s: String = chars.get(start..=i).unwrap_or(&[]).iter().collect();
                    let s = s.trim().to_string();
                    if !s.is_empty() {
                        out.push(s);
                    }
                    start = i + 1;
                }
            }
        }
        i += 1;
    }
    let rest: String = chars.get(start..).unwrap_or(&[]).iter().collect();
    let rest = rest.trim();
    if !rest.is_empty() {
        out.push(rest.to_string());
    }
    out
}

pub fn words(s: &str) -> usize {
    s.split_whitespace().count()
}

/// The doc's own spelling for words it also writes in capitals: "OKTA" → "Okta" when "Okta" appears.
pub fn casing_map(all_text: &str) -> BTreeMap<String, String> {
    static WORD: LazyLock<Option<Regex>> = LazyLock::new(|| re(r"\b[A-Za-z][A-Za-z]+\b"));
    let Some(word) = WORD.as_ref() else {
        return BTreeMap::new();
    };
    let mut mixed: BTreeMap<String, String> = BTreeMap::new();
    let mut caps: Vec<String> = Vec::new();
    for m in word.find_iter(all_text) {
        let w = m.as_str();
        if w.len() >= 3 && w.chars().all(|c| c.is_ascii_uppercase()) {
            caps.push(w.to_string());
        } else if w.chars().next().is_some_and(char::is_uppercase)
            && w.chars().skip(1).any(char::is_lowercase)
        {
            mixed
                .entry(w.to_ascii_uppercase())
                .or_insert_with(|| w.to_string());
        }
    }
    caps.into_iter()
        .filter_map(|c| mixed.get(&c).map(|m| (c, m.clone())))
        .collect()
}

struct Rewrite {
    re: Regex,
    to: &'static str,
}

/// Staff-voice documentation into second-person narration. Order matters: specific forms first.
static REWRITES: LazyLock<Vec<Rewrite>> = LazyLock::new(|| {
    let rules: &[(&str, &str)] = &[
        (r"\*{2,}", ""),
        (r"\s*\(\s*https?://([^)\s]+?)/?\s*\)", " at $1"),
        (r"https?://([^\s/]+)/?", "$1"),
        (r"\band\s*/\s*or\b", "or"),
        (r"\band or\b", "or"),
        (
            r"(?i)^users (?:that|who) do not have (.+?) (?:will|are going to) (.+)$",
            "If you don't have $1, you'll $2",
        ),
        (
            r"(?i)^users (?:that|who) (?:have|use) (.+?) will (.+)$",
            "If you have $1, you'll $2",
        ),
        (
            r"(?i)^users with (.+?) are already set ?up\.?$",
            "If you already use $1, you're all set.",
        ),
        (
            r"(?i)^users without (.+?) (?:already )?set ?up\.?$",
            "If you don't have $1 set up yet.",
        ),
        (r"(?i)^(?:all )?users (?:will|must) ", "You'll "),
        (
            r"(?i)\busers (?:that|who) do not have (.+?) (?:configured |set up )?will be\b",
            "if you don't have $1, you'll be",
        ),
        (r"\bThey will not\b", "You won't"),
        (r"\bthey will not\b", "you won't"),
        (r"\bThey will\b", "You'll"),
        (r"\bthey will\b", "you'll"),
        (r"\bThey\b", "You"),
        (r"\bthey\b", "you"),
        (r"\btheir\b", "your"),
        (r"\bTheir\b", "Your"),
        (r"\bthem\b", "you"),
        (
            r"\b(during|at) your first login\b",
            "the first time you sign in",
        ),
        (r"\bduring login\b", "when you sign in"),
        (r"\blogin to\b", "sign in to"),
        (r"\blog in to\b", "sign in to"),
        (r"\bto login\b", "to sign in"),
        (
            r"\b(already|to|not|if|can|cannot|will) setup\b",
            "$1 set up",
        ),
        (r"\bbe prompted\b", "be asked"),
        (r"\bremind (\w+) of the need to\b", "remind $1 to"),
        (r"\byou will not\b", "you won't"),
        (r"\byou will\b", "you'll"),
        (r"\bYou will\b", "You'll"),
        (r"\byou are\b", "you're"),
        (r"\bdo not\b", "don't"),
        (r"\bDo not\b", "Don't"),
        (r"\bcannot\b", "can't"),
        (r"\bwill not\b", "won't"),
        (r"\s+([,.;:!?])(\s|$)", "$1$2"),
        (r"\s{2,}", " "),
    ];
    rules
        .iter()
        .filter_map(|(p, to)| re(p).map(|re| Rewrite { re, to }))
        .collect()
});

/// Rewrite one sentence for narration: second person, contractions, the doc's own casing, no URL
/// schemes, a full stop at the end.
pub fn speakable(sentence: &str, casing: &BTreeMap<String, String>) -> String {
    let mut s = sentence.trim().to_string();
    for r in REWRITES.iter() {
        s = r.re.replace_all(&s, r.to).into_owned();
    }
    for (caps, mixed) in casing {
        if let Some(r) = re(&format!(r"\b{}\b", regex::escape(caps))) {
            s = r.replace_all(&s, mixed.as_str()).into_owned();
        }
    }
    let s = s
        .trim()
        .trim_start_matches(['-', '•', '*', ' '])
        .trim()
        .to_string();
    let mut s = capitalize(&s);
    if !s.is_empty() && !s.ends_with(['.', '!', '?', ':']) {
        s.push('.');
    }
    if s.ends_with(':') {
        s.pop();
        s.push('.');
    }
    s
}

pub fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// A short version for a callout: the first clause as the head, the rest (capped) as the sub-line.
pub fn condense(sentence: &str, head_words: usize, sub_words: usize) -> (String, String) {
    let s = sentence.trim().trim_end_matches('.').trim();
    // a little over the limit reads better whole than cut
    if words(s) <= head_words + 4 {
        return (s.to_string(), String::new());
    }
    let cut_at = [
        ", but ", ", and ", ", ", " - ", "; ", ": ", " but ", " and ",
    ]
    .iter()
    .filter_map(|sep| {
        let mut best = None;
        for (pos, _) in s.match_indices(sep) {
            let before = s.get(..pos).map(words).unwrap_or(0);
            if (3..=head_words).contains(&before) {
                best = Some((pos, sep.len()));
            }
        }
        best
    })
    .next();
    let (head, rest) = match cut_at {
        Some((pos, len)) => (
            s.get(..pos).unwrap_or(s).to_string(),
            s.get(pos + len..).unwrap_or("").to_string(),
        ),
        None => {
            // no clause to cut at: keep a clean head and leave the detail to the narration
            let w: Vec<&str> = s.split_whitespace().collect();
            // never strand a name: "…need to set up" takes "Okta Verify" with it
            let mut take = head_words;
            while take < head_words + 3
                && w.get(take)
                    .is_some_and(|x| x.chars().next().is_some_and(char::is_uppercase))
            {
                take += 1;
            }
            let head = trim_dangling(&w.iter().take(take).copied().collect::<Vec<_>>());
            return (capitalize(&head), String::new());
        }
    };
    let rw: Vec<&str> = rest.split_whitespace().collect();
    let sub = if rw.len() > sub_words {
        format!(
            "{}…",
            rw.iter()
                .take(sub_words)
                .copied()
                .collect::<Vec<_>>()
                .join(" ")
        )
    } else {
        rest
    };
    (capitalize(head.trim()), capitalize(sub.trim()))
}

const DANGLING: &[&str] = &[
    "a", "an", "the", "to", "of", "for", "or", "and", "with", "in", "on", "at", "by", "your",
    "you", "be", "will", "can", "from", "as", "is", "are", "that", "which", "if", "but", "you'll",
];

/// Words joined, without the little words a cut leaves dangling at the end.
fn trim_dangling(w: &[&str]) -> String {
    let mut end = w.len();
    while end > 1
        && w.get(end - 1)
            .is_some_and(|x| DANGLING.contains(&x.to_lowercase().trim_matches(',')))
    {
        end -= 1;
    }
    w.get(..end)
        .unwrap_or(w)
        .join(" ")
        .trim_end_matches(',')
        .to_string()
}

/// Cut text at a word boundary to at most `max` characters, without dangling little words.
pub fn cut_words(s: &str, max: usize) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut len = 0usize;
    for w in s.split_whitespace() {
        let add = w.chars().count() + usize::from(!out.is_empty());
        if len + add > max {
            break;
        }
        len += add;
        out.push(w);
    }
    trim_dangling(&out)
}

/// Showrig draws titles on one line in Inter Bold with no wrapping: a lower third at 132 px from
/// x = 150 (about 1730 px wide), a centred title at 150 px (118 px past 14 characters) across about
/// 1840 px. Widths here are in ems of Inter Bold, from approximate advance widths per character class.
pub const LOWER_THIRD_EM: f64 = 1730.0 / 132.0;

pub fn centred_title_em(s: &str) -> f64 {
    if s.chars().count() > 14 {
        1840.0 / 118.0
    } else {
        1840.0 / 150.0
    }
}

pub fn title_em(s: &str) -> f64 {
    s.chars()
        .map(|c| match c {
            ' ' => 0.26,
            '.' | ',' | ':' | ';' | '\'' | '!' | '|' => 0.28,
            'i' | 'j' | 'l' => 0.27,
            'f' | 't' | 'r' => 0.4,
            'm' | 'w' => 0.88,
            'M' | 'W' => 0.98,
            'I' => 0.3,
            c if c.is_ascii_digit() => 0.62,
            c if c.is_uppercase() => 0.7,
            c if c.is_lowercase() => 0.59,
            _ => 0.6,
        })
        .sum::<f64>()
        - 0.01 * s.chars().count() as f64
}

/// How far a title runs past what Showrig can draw, as a percentage; None when it fits.
/// `centred` is for title cards (intro, cards, outro), otherwise it is a lower third.
pub fn title_overflow(title: &str, centred: bool) -> Option<u32> {
    let max = if centred {
        centred_title_em(title)
    } else {
        LOWER_THIRD_EM
    };
    let w = title_em(title);
    (w > max).then(|| ((w / max - 1.0) * 100.0).ceil() as u32)
}

/// Cut a title at a word boundary so it fits `max_em`, without dangling little words or commas.
pub fn fit_title(s: &str, max_em: f64) -> String {
    if title_em(s) <= max_em {
        return s.to_string();
    }
    let mut out: Vec<&str> = Vec::new();
    for w in s.split_whitespace() {
        let candidate = if out.is_empty() {
            w.to_string()
        } else {
            format!("{} {w}", out.join(" "))
        };
        if title_em(&candidate) > max_em {
            break;
        }
        out.push(w);
    }
    let t = trim_dangling(&out);
    t.trim_end_matches([',', ';', ':']).to_string()
}

pub fn tone(s: &str) -> Tone {
    let l = format!(" {} ", s.to_lowercase());
    let has = |ws: &[&str]| ws.iter().any(|w| l.contains(w));
    if has(&[
        " not ",
        "no longer",
        "won't",
        "can't",
        "cannot",
        "retire",
        " removed",
        " unavailable",
    ]) {
        Tone::Bad
    } else if has(&[
        " skip", " until ", " before ", "required", " must ", "deadline", "remind", " need ",
        " asked ",
    ]) {
        Tone::Warn
    } else if has(&[
        "already",
        "able to",
        " can ",
        "all set",
        "continue",
        "available",
    ]) {
        Tone::Ok
    } else {
        Tone::Info
    }
}

/// A file- and id-safe slug.
pub fn slug(s: &str, max: usize) -> String {
    let mut out = String::new();
    for c in s.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.len() >= max {
            break;
        }
    }
    out.trim_matches('-').to_string()
}

/// A date the document mentions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateHit {
    /// As written: "Nov 2-15, 2026".
    pub text: String,
    /// For the screen: "November 2–15".
    pub label: String,
    /// For ordering: month * 100 + day.
    pub key: u32,
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

static DATE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    re(
        r"(?i)\b(jan(?:uary)?|feb(?:ruary)?|mar(?:ch)?|apr(?:il)?|may|june?|july?|aug(?:ust)?|sep(?:t(?:ember)?)?|oct(?:ober)?|nov(?:ember)?|dec(?:ember)?)\.?\s+(\d{1,2})(?:st|nd|rd|th)?(?:\s*(?:-|–|to|through)\s*(\d{1,2})(?:st|nd|rd|th)?)?(?:,?\s*(\d{4}))?\b",
    )
});

pub fn dates(s: &str) -> Vec<DateHit> {
    let Some(date) = DATE.as_ref() else {
        return Vec::new();
    };
    date.captures_iter(s)
        .filter_map(|c| {
            let m = c.get(1)?.as_str().to_lowercase();
            let abbr = m.get(..3)?.to_string();
            let month = MONTHS
                .iter()
                .position(|full| full.to_lowercase().starts_with(&abbr))?;
            let day: u32 = c.get(2)?.as_str().parse().ok()?;
            if !(1..=31).contains(&day) {
                return None;
            }
            let name = MONTHS.get(month)?;
            let label = match c.get(3) {
                Some(end) => format!("{name} {day}–{}", end.as_str()),
                None => format!("{name} {day}"),
            };
            Some(DateHit {
                text: c.get(0)?.as_str().to_string(),
                label,
                key: (month as u32 + 1) * 100 + day,
            })
        })
        .collect()
}

const IMPERATIVES: &[&str] = &[
    "click", "open", "select", "choose", "go", "enter", "type", "sign", "navigate", "press", "tap",
    "install", "download", "copy", "paste", "send", "find", "set", "add", "remove", "check",
    "make", "save", "close", "log", "visit", "scan", "follow", "use", "turn", "connect", "restart",
    "launch", "create", "email", "call", "verify", "confirm", "update",
];

pub fn imperative(s: &str) -> bool {
    let first = s
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(|c: char| !c.is_alphabetic())
        .to_lowercase();
    IMPERATIVES.contains(&first.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_sentences() {
        let s = sentences(
            "Starts Nov. 2 for everyone. See e.g. the dashboard! Done? Yes (https://a.b.edu).",
        );
        assert_eq!(
            s,
            vec![
                "Starts Nov. 2 for everyone.",
                "See e.g. the dashboard!",
                "Done?",
                "Yes (https://a.b.edu)."
            ]
        );
    }

    #[test]
    fn rewrites_to_second_person() {
        let casing = casing_map("OKTA Verify and Okta Verify");
        assert_eq!(
            speakable(
                "Users with OKTA Verify and/or Google Authenticator are already set up.",
                &casing
            ),
            "If you already use Okta Verify or Google Authenticator, you're all set."
        );
        assert_eq!(
            speakable(
                "They will be able to login to any applications which require password only",
                &casing
            ),
            "You'll be able to sign in to any applications which require password only."
        );
        assert_eq!(
            speakable(
                "Okta dashboard login (https://cloud.example.edu) continues to not require a second factor",
                &casing
            ),
            "Okta dashboard login at cloud.example.edu continues to not require a second factor."
        );
    }

    #[test]
    fn finds_dates() {
        let d = dates("from Nov 2-15, 2026, then from November 16th onward");
        let labels: Vec<_> = d.iter().map(|d| (d.label.as_str(), d.key)).collect();
        assert_eq!(labels, vec![("November 2–15", 1102), ("November 16", 1116)]);
    }

    #[test]
    fn condenses() {
        let (h, s) = condense(
            "You'll be able to sign in to any app that needs only a password and nothing else at all today",
            9,
            14,
        );
        assert_eq!(
            (h.as_str(), s.as_str()),
            ("You'll be able to sign in to any app", "")
        );
        assert_eq!(
            cut_words(
                "If you already use Okta Verify or Google Authenticator today",
                38
            ),
            "If you already use Okta Verify"
        );
        let (h, s) = condense(
            "You can choose Remind me later up to three times, and then setup is required",
            10,
            14,
        );
        assert_eq!(
            (h.as_str(), s.as_str()),
            (
                "You can choose Remind me later up to three times",
                "Then setup is required"
            )
        );
    }
}
