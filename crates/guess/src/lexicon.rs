//! Pronunciations for the voice (Showrig's `voice.say`): acronyms spelled out, web addresses and
//! emails read with "dot" and "at", abbreviated months and day ranges read in full.
//!
//! Showrig applies `say` as plain, case-sensitive substring replacements in order, so longer keys
//! come first and a key is never added when it would split a longer capitalised word.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;

/// Capitalised words people say as words, not letter by letter.
const SAY_AS_WORD: &[&str] = &[
    "NASA", "SCUBA", "LASER", "RADAR", "PIN", "SIM", "GIF", "JPEG", "RAM", "ROM", "WAN", "LAN",
    "SaaS", "OKTA", "COVID", "FAQ", "ASAP", "AIDS", "NATO", "UNESCO", "POSIX", "SQL", "WIFI",
    "CAPTCHA", "OAUTH", "BIOS", "ZIP", "CAD", "RAID",
];

/// Top-level domains read as letters.
const SPELLED_TLDS: &[&str] = &["edu", "io", "ai", "uk", "us", "ca", "tv"];

static ACRONYM: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"\b[A-Z][A-Z0-9]{1,5}s?\b").ok());
static DOMAIN: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"\b(?:[a-zA-Z0-9-]+@)?(?:[a-zA-Z0-9-]+\.)+(?:com|org|net|edu|gov|io|ai|co|uk|us|ca|tv|app|dev)\b").ok()
});
static MONTH_ABBR: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"\b(Jan|Feb|Mar|Apr|Jun|Jul|Aug|Sep|Sept|Oct|Nov|Dec)\.? (\d)").ok()
});
static RANGE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"\b(\d{1,2})-(\d{1,2})\b").ok());

fn spell(word: &str) -> String {
    let (core, plural) = match word.strip_suffix('s') {
        Some(c)
            if c.chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()) =>
        {
            (c, true)
        }
        _ => (word, false),
    };
    let letters: Vec<String> = core.chars().map(|c| c.to_string()).collect();
    let mut out = letters.join(" ");
    if plural {
        out.push('s');
    }
    out
}

fn speak_domain(d: &str) -> String {
    let (user, host) = match d.split_once('@') {
        Some((u, h)) => (Some(u), h),
        None => (None, d),
    };
    let parts: Vec<&str> = host.split('.').collect();
    let n = parts.len();
    let spoken: Vec<String> = parts
        .iter()
        .enumerate()
        .map(|(i, p)| {
            if i + 1 == n && SPELLED_TLDS.contains(&p.to_ascii_lowercase().as_str()) {
                spell(&p.to_ascii_uppercase())
            } else {
                p.to_string()
            }
        })
        .collect();
    let host = spoken.join(" dot ");
    match user {
        Some(u) => format!("{u} at {host}"),
        None => host,
    }
}

const MONTH_FULL: [(&str, &str); 12] = [
    ("Jan", "January"),
    ("Feb", "February"),
    ("Mar", "March"),
    ("Apr", "April"),
    ("Jun", "June"),
    ("Jul", "July"),
    ("Aug", "August"),
    ("Sep", "September"),
    ("Sept", "September"),
    ("Oct", "October"),
    ("Nov", "November"),
    ("Dec", "December"),
];

/// The pronunciations for these narration lines, with the person's own entries applied last
/// (an empty value removes an automatic entry). Longest keys first.
pub fn say(lines: &[&str], overrides: &BTreeMap<String, String>) -> Vec<(String, String)> {
    let mut auto: BTreeMap<String, String> = BTreeMap::new();
    let all_caps_words: Vec<String> = lines
        .iter()
        .flat_map(|l| l.split(|c: char| !c.is_alphanumeric()))
        .filter(|w| {
            w.len() >= 2
                && w.chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        })
        .map(str::to_string)
        .collect();
    for line in lines {
        if let Some(d) = DOMAIN.as_ref() {
            for m in d.find_iter(line) {
                auto.insert(m.as_str().to_string(), speak_domain(m.as_str()));
            }
        }
        if let Some(a) = ACRONYM.as_ref() {
            for m in a.find_iter(line) {
                let w = m.as_str();
                let core = w.strip_suffix('s').filter(|c| c.len() >= 2).unwrap_or(w);
                if SAY_AS_WORD.iter().any(|s| s.eq_ignore_ascii_case(core))
                    || core.chars().all(|c| c.is_ascii_digit())
                {
                    continue;
                }
                if !core
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
                {
                    continue;
                }
                // four letters or more with a vowel reads as a word (NOTE, EDIT, OKTA); short ones and
                // consonant clusters are said letter by letter (IT, MFA, HTML)
                if core.len() >= 4
                    && core
                        .chars()
                        .any(|c| matches!(c, 'A' | 'E' | 'I' | 'O' | 'U'))
                {
                    continue;
                }
                // never add a key that sits inside a longer capitalised word, e.g. IT inside EDIT
                let splits_other = all_caps_words
                    .iter()
                    .any(|o| o.len() > w.len() && o.contains(w) && o != w);
                if splits_other {
                    continue;
                }
                auto.insert(w.to_string(), spell(w));
            }
        }
        if let Some(m) = MONTH_ABBR.as_ref() {
            for c in m.captures_iter(line) {
                let (Some(whole), Some(abbr)) = (c.get(0), c.get(1)) else {
                    continue;
                };
                let digit_len = c.get(2).map(|d| d.as_str().len()).unwrap_or(0);
                let key = whole
                    .as_str()
                    .get(..whole.as_str().len().saturating_sub(digit_len))
                    .unwrap_or("")
                    .to_string();
                if let Some((_, full)) = MONTH_FULL.iter().find(|(a, _)| *a == abbr.as_str()) {
                    auto.insert(key, format!("{full} "));
                }
            }
        }
        if let Some(r) = RANGE.as_ref() {
            for c in r.captures_iter(line) {
                if let (Some(w), Some(a), Some(b)) = (c.get(0), c.get(1), c.get(2)) {
                    auto.insert(
                        w.as_str().to_string(),
                        format!("{} to {}", a.as_str(), b.as_str()),
                    );
                }
            }
        }
    }
    for (k, v) in overrides {
        if v.is_empty() {
            auto.remove(k);
        } else {
            auto.insert(k.clone(), v.clone());
        }
    }
    let mut out: Vec<(String, String)> = auto.into_iter().collect();
    out.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pronunciations() {
        let lines = [
            "Campus IT retires SMS codes for MFA on Nov 16.",
            "Sign in at cloud.example.edu or mail help@example.edu.",
            "Nov 2-15 is the reminder window.",
            "EDIT the NASA file.",
        ];
        let say = say(&lines, &BTreeMap::new());
        let get = |k: &str| say.iter().find(|(a, _)| a == k).map(|(_, b)| b.as_str());
        assert_eq!(
            get("cloud.example.edu"),
            Some("cloud dot example dot E D U")
        );
        assert_eq!(get("help@example.edu"), Some("help at example dot E D U"));
        assert_eq!(get("MFA"), Some("M F A"));
        assert_eq!(get("SMS"), Some("S M S"));
        assert_eq!(get("Nov "), Some("November "));
        assert_eq!(get("2-15"), Some("2 to 15"));
        assert_eq!(get("NASA"), None);
        // IT would split EDIT, so it is left for the person to decide
        assert_eq!(get("IT"), None);
        assert_eq!(get("EDIT"), None);
        // longest keys first
        assert_eq!(
            say.first().map(|(k, _)| k.as_str()),
            Some("cloud.example.edu")
        );
    }

    #[test]
    fn it_alone_is_spelled() {
        let say = say(&["Call Campus IT."], &BTreeMap::new());
        assert_eq!(say, vec![("IT".to_string(), "I T".to_string())]);
    }
}
