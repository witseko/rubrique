//! Locate and preserve the passage a recording belongs to.

use std::ops::Range;

use crate::markdown::blocks::{parse_blocks, parse_source_blocks};
use crate::markdown::html::html_body;
use crate::markdown::notes::{self, find_notes, voice_note_comment};

/// A byte position and the source needed to verify it when recording ends.
#[derive(Debug)]
pub(super) struct VoiceAnchor {
    prefix: String,
    offset: usize,
    quote: String,
}

impl VoiceAnchor {
    /// Locate a selection and verify that inserting a comment keeps its Markdown intact.
    pub(super) fn new(
        source: &str,
        quote: &str,
        editor_range: Option<&Range<usize>>,
        preview_block: Option<&Range<usize>>,
    ) -> Result<Self, &'static str> {
        if quote.is_empty() {
            return Err("Select some text to record a voice note.");
        }
        let (block, offset) = if let Some(range) = editor_range {
            let block = parse_source_blocks(source)
                .into_iter()
                .find(|block| block.range.start <= range.start && block.range.end >= range.end)
                .ok_or("Select text within one Markdown block.")?;
            (block.range, range.end)
        } else {
            let block = preview_block.ok_or("Select text within one Markdown block.")?;
            let text = source.get(block.clone()).ok_or("The passage changed. Select it again.")?;
            (block.clone(), block.start + unique_end(text, quote)?)
        };
        let text = &source[block.clone()];
        let at = offset - block.start;
        let mut annotated = text.to_string();
        annotated.insert_str(at, &voice_note_comment(quote, "recording.wav"));
        if !find_notes(&annotated).iter().any(|note| note.range.start == at)
            || html_body(&parse_blocks(text)) != html_body(&parse_blocks(&annotated))
        {
            return Err("Select text where a voice note can be added without changing the Markdown.");
        }
        Ok(Self { prefix: source[..block.end].to_string(), offset, quote: quote.to_string() })
    }

    /// Return the insertion point and comment if the original passage is still in place.
    pub(super) fn insertion(&self, source: &str, url: &str) -> Option<(Range<usize>, String)> {
        source.starts_with(&self.prefix).then(|| (self.offset..self.offset, voice_note_comment(&self.quote, url)))
    }
}

/// Locate one visible occurrence within the preview block.
fn unique_end(block: &str, quote: &str) -> Result<usize, &'static str> {
    let comments = find_notes(block);
    let mut matches = notes::locate_matches(block, quote)
        .filter(|range| !comments.iter().any(|note| note.range.contains(&range.start)));
    let first = matches.next().ok_or("Cannot find the selected text in the source. Try selecting it in Edit mode.")?;
    if matches.next().is_some() {
        return Err("This text repeats in the same block. Select the occurrence in Edit mode.");
    }
    Ok(first.end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::notes::NoteContent;

    #[test]
    fn attaches_to_the_selected_block_and_survives_edits_after_it() {
        let source = "Bonjour café.\n\nBonjour **café**.\n\nNext block";
        let blocks = parse_source_blocks(source);
        let anchor = VoiceAnchor::new(source, "café", None, Some(&blocks[1].range)).unwrap();
        let mut edited = source.replace("Next block", "Changed");
        let (range, comment) = anchor.insertion(&edited, "memos/test.wav").unwrap();
        edited.replace_range(range, &comment);
        assert_eq!(edited, format!("Bonjour café.\n\nBonjour **café{comment}**.\n\nChanged"));
        let notes = find_notes(&edited);
        assert_eq!(notes[0].content, NoteContent::Voice("memos/test.wav".to_string()));
        assert!(anchor.insertion(&source.replace("Bonjour", "Salut"), "memos/test.wav").is_none());
    }

    #[test]
    fn editor_range_disambiguates_repeated_text() {
        let source = "same same";
        assert!(VoiceAnchor::new(source, "same", None, Some(&(0..source.len()))).is_err());
        let anchor = VoiceAnchor::new(source, "same", Some(&(5..9)), None).unwrap();
        assert_eq!(anchor.insertion(source, "memo.wav").unwrap().0, 9..9);
    }

    #[test]
    fn refuses_to_insert_inside_code_or_across_blocks() {
        let source = "Use `code`.\n\nNext";
        assert!(VoiceAnchor::new(source, "code", Some(&(5..9)), None).is_err());
        assert!(VoiceAnchor::new(source, "Use code Next", Some(&(0..source.len())), None).is_err());
    }
}
