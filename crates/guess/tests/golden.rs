#![allow(
    clippy::panic,
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used
)] // test code may panic
//! Golden tests: the guess for the synthetic fixtures (tests/fixtures, made by make_fixtures.py),
//! determinism, and an opt-in check against a real document (EXPLAINRIG_REAL_DOC).

use std::path::PathBuf;

use explainrig_guess::{DocType, Layout, Project, Source, guess};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn guessed(path: &std::path::Path) -> Project {
    let doc = explainrig_doc::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    guess(
        &doc,
        Source {
            path: path.display().to_string(),
            hash: String::new(),
        },
    )
}

fn layouts(p: &Project) -> Vec<Layout> {
    p.scenes.iter().map(|s| s.layout).collect()
}

#[test]
fn change_notice_from_a_loosely_styled_docx() {
    let p = guessed(&fixture("change-notice.docx"));
    assert_eq!(p.doc_type, DocType::ChangeNotice);
    assert_eq!(p.title, "Retiring Paper Timesheets");
    assert_eq!(
        layouts(&p),
        vec![
            Layout::Title,
            Layout::Timeline,
            Layout::Screenshot,
            Layout::Screenshot,
            Layout::Title
        ]
    );
    let timeline = &p.scenes[1];
    let stops: Vec<&str> = timeline.points.iter().map(|pt| pt.head.as_str()).collect();
    assert_eq!(stops, vec!["March 3–14", "March 15"]);
    assert_eq!(p.dropped.len(), 1, "the Background label is dropped");
    assert_eq!(p.dropped[0].name, "Background");
    assert_eq!(p.scenes[2].image.as_deref(), Some("image1"));
    assert_eq!(p.scenes[3].image.as_deref(), Some("image2"));
    // the staff-voice heading becomes second person, and every rules line is marked draft
    assert!(
        p.scenes[2].lines[0]
            .text
            .starts_with("If you already use the Timekeeper app, you're all set"),
        "{:?}",
        p.scenes[2].lines
    );
    assert!(p.scenes.iter().flat_map(|s| &s.lines).all(|l| l.draft));
    assert!(
        p.scenes
            .last()
            .is_some_and(|s| s.title.starts_with("Set up Timekeeper")),
        "the call to action closes the film"
    );
}

#[test]
fn procedure_from_markdown() {
    let p = guessed(&fixture("procedure.md"));
    assert_eq!(p.doc_type, DocType::Procedure);
    let ids: Vec<&str> = p.scenes.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(
        ids,
        vec![
            "intro",
            "before-you-start",
            "step1",
            "step2",
            "step3",
            "step4",
            "checklist",
            "outro"
        ]
    );
    assert_eq!(
        p.scenes[4].layout,
        Layout::Screenshot,
        "step 3 has the picture"
    );
    assert_eq!(p.scenes[2].layout, Layout::Steps);
    assert_eq!(p.scenes[2].focus, Some(0));
}

#[test]
fn guess_is_deterministic() {
    for name in ["change-notice.docx", "procedure.md"] {
        let a = serde_json::to_string(&guessed(&fixture(name))).unwrap_or_default();
        let b = serde_json::to_string(&guessed(&fixture(name))).unwrap_or_default();
        assert!(!a.is_empty());
        assert_eq!(a, b, "{name}");
    }
}

/// Opt in with EXPLAINRIG_REAL_DOC=/path/to/Sunsetting SMS-Voice - User Experience.docx
#[test]
fn real_change_notice_matches_the_hand_made_film() {
    let Ok(path) = std::env::var("EXPLAINRIG_REAL_DOC") else {
        return;
    };
    let p = guessed(std::path::Path::new(&path));
    assert_eq!(p.doc_type, DocType::ChangeNotice);
    assert_eq!(
        layouts(&p),
        vec![
            Layout::Title,
            Layout::Timeline,
            Layout::Screenshot,
            Layout::Screenshot,
            Layout::Screenshot,
            Layout::Title
        ]
    );
    assert_eq!(
        p.dropped
            .iter()
            .map(|d| d.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Background"]
    );
    assert_eq!(
        p.scenes.last().map(|s| s.title.as_str()),
        Some("Set up Okta Verify today")
    );
}
