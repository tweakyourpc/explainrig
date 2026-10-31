//! Explainrig's best guess: a document's blocks in, a storyboard to review out.
//!
//! Rules only, no models: sections from headings or bold lead-ins, a document type from scored
//! features, a scene recipe per type, narration drafts from the document's own sentences
//! rewritten into the second person, and pronunciations for the voice.

pub mod lexicon;
pub mod model;
mod rules;
pub mod text;

pub use model::*;
pub use rules::guess;
