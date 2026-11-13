//! Search: matches of a query in a text (without case by default), in
//! every text file under the explorer's folders, and replacing them. Pure
//! functions, apart from the file reading in [`search_files`].

use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Files larger than this are skipped in an all-files search.
const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// Longest excerpt shown for a hit, in characters.
const EXCERPT_CHARS: usize = 100;
/// Characters kept before a match that would fall past the end of the
/// excerpt: the excerpt then starts this far before the match instead of at
/// the start of the line, so the match stays in view.
const EXCERPT_CONTEXT_CHARS: usize = 30;
/// Marks text cut from the start or the end of an excerpt.
const ELLIPSIS: char = '\u{2026}';

/// How a query is compared with the text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SearchOptions {
    /// Letters must match in case. Off compares without case.
    pub match_case: bool,
    /// A match must not touch a letter, digit or `_` on either side.
    pub whole_word: bool,
}

/// Byte ranges of every occurrence of `query` in `text`, in order, without
/// overlaps. An empty query matches nothing.
pub fn find_all(text: &str, query: &str, options: SearchOptions) -> Vec<Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    let ranges: Vec<Range<usize>> = if options.match_case {
        text.match_indices(query).map(|(start, found)| start..start + found.len()).collect()
    } else {
        let lowered = Lowered::new(text);
        let query = query.to_lowercase();
        lowered
            .text
            .match_indices(query.as_str())
            .map(|(start, found)| lowered.original(start..start + found.len()))
            .collect()
    };
    if options.whole_word {
        return ranges.into_iter().filter(|range| is_whole_word(text, range)).collect();
    }
    ranges
}

/// True when neither the character before `range` nor the one after it is
/// a word character (a letter, a digit or `_`).
fn is_whole_word(text: &str, range: &Range<usize>) -> bool {
    let before = text[..range.start].chars().next_back();
    let after = text[range.end..].chars().next();
    !before.is_some_and(is_word_char) && !after.is_some_and(is_word_char)
}

fn is_word_char(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

/// `text` with every one of `matches` (in order, without overlaps, as
/// [`find_all`] gives them) replaced by `with`.
pub fn replace_all(text: &str, matches: &[Range<usize>], with: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut copied_to = 0;
    for range in matches {
        result.push_str(&text[copied_to..range.start]);
        result.push_str(with);
        copied_to = range.end;
    }
    result.push_str(&text[copied_to..]);
    result
}

/// `text` in lower case, with the way back to the original byte offsets
/// (lower-casing can change the byte length of a character).
struct Lowered {
    text: String,
    /// For each byte of `text`, the original byte it came from; one more
    /// entry for the end.
    origin: Vec<usize>,
}

impl Lowered {
    fn new(text: &str) -> Self {
        let mut lowered = String::with_capacity(text.len());
        let mut origin = Vec::with_capacity(text.len() + 1);
        for (index, character) in text.char_indices() {
            for lower in character.to_lowercase() {
                let before = lowered.len();
                lowered.push(lower);
                origin.extend(std::iter::repeat_n(index, lowered.len() - before));
            }
        }
        origin.push(text.len());
        Lowered { text: lowered, origin }
    }

    /// The original byte range for a range in the lowered text.
    fn original(&self, range: Range<usize>) -> Range<usize> {
        self.origin[range.start]..self.origin[range.end]
    }
}

/// Where a query was found in a folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileHit {
    pub path: PathBuf,
    /// The explorer folder the file was found under.
    pub root: PathBuf,
    pub kind: HitKind,
}

/// What matched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HitKind {
    /// The file's name contains the query.
    Name,
    /// A line of the file contains the query.
    Text {
        /// 1-based line number.
        line: usize,
        /// The line, trimmed and cut to [`EXCERPT_CHARS`] around the match,
        /// with an ellipsis where text was cut.
        excerpt: String,
        /// Byte range of the match in `excerpt`, to highlight it.
        highlight: Range<usize>,
        /// Byte range of the match in the whole file.
        range: Range<usize>,
    },
}

/// An all-files search.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchResults {
    pub hits: Vec<FileHit>,
    pub files_searched: usize,
    /// Stopped at `limit` hits; there may be more.
    pub truncated: bool,
}

impl SearchResults {
    /// Files with at least one hit.
    pub fn file_count(&self) -> usize {
        let mut paths: Vec<&Path> = self.hits.iter().map(|hit| hit.path.as_path()).collect();
        paths.dedup();
        paths.len()
    }
}

/// What an all-files search looks for.
#[derive(Clone, Copy, Debug)]
pub struct Query<'a> {
    pub text: &'a str,
    /// How the text of the files is compared. File names are always
    /// compared without case.
    pub options: SearchOptions,
}

/// Search the text files under `roots` (hidden entries skipped) for `query`,
/// stopping after `limit` hits. Files come in explorer order; name hits
/// before text hits within a file. Setting `cancel` (from another thread)
/// stops the search at the next file, with the hits found so far: a search
/// nobody waits for any more should not keep reading a large folder.
pub fn search_files(roots: &[PathBuf], query: Query<'_>, limit: usize, cancel: &AtomicBool) -> SearchResults {
    let mut results = SearchResults::default();
    if query.text.trim().is_empty() {
        return results;
    }
    for root in roots {
        search_folder(root, root, query, limit, cancel, &mut results);
        if results.truncated || cancel.load(Ordering::Relaxed) {
            break;
        }
    }
    results
}

fn search_folder(
    folder: &Path,
    root: &Path,
    query: Query<'_>,
    limit: usize,
    cancel: &AtomicBool,
    results: &mut SearchResults,
) {
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    let mut entries: Vec<fs::DirEntry> = entries.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name().to_string_lossy().to_lowercase());
    for entry in entries {
        if results.truncated || cancel.load(Ordering::Relaxed) {
            return;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let Ok(metadata) = fs::metadata(&path) else {
            continue;
        };
        if metadata.is_dir() {
            // Links to folders are not followed, so a loop cannot happen.
            if !entry.file_type().is_ok_and(|kind| kind.is_symlink()) {
                search_folder(&path, root, query, limit, cancel, results);
            }
            continue;
        }
        let text = if metadata.len() > MAX_FILE_BYTES { None } else { fs::read_to_string(&path).ok() };
        search_file(&path, root, text.as_deref(), query, limit, results);
    }
}

/// Add the hits of one file: its name, then its text (when it was read and
/// is text).
fn search_file(
    path: &Path,
    root: &Path,
    text: Option<&str>,
    query: Query<'_>,
    limit: usize,
    results: &mut SearchResults,
) {
    let name = path.file_name().map(|name| name.to_string_lossy().to_lowercase()).unwrap_or_default();
    if name.contains(&query.text.to_lowercase()) {
        push_hit(results, FileHit { path: path.to_path_buf(), root: root.to_path_buf(), kind: HitKind::Name }, limit);
    }
    // Encrypted scratches: only their names are searched.
    let Some(text) = text.filter(|text| !text.contains('\0') && !crate::crypto::is_encrypted(text.as_bytes())) else {
        return;
    };
    results.files_searched += 1;
    for range in find_all(text, query.text, query.options) {
        if results.truncated {
            return;
        }
        let (line, excerpt, highlight) = excerpt_of(text, &range);
        let kind = HitKind::Text { line, excerpt, highlight, range };
        push_hit(results, FileHit { path: path.to_path_buf(), root: root.to_path_buf(), kind }, limit);
    }
}

/// Most files whose text is downloaded for one remote search.
const MAX_REMOTE_TEXT_FILES: usize = 200;

/// [`search_files`] for remote folders (mirror paths): the host lists the
/// files and finds the ones containing the query, then those are
/// downloaded and searched here, so hits look the same as local ones.
///
/// # Errors
///
/// A host cannot be reached.
pub fn search_remote_files(roots: &[PathBuf], query: Query<'_>, limit: usize) -> std::io::Result<SearchResults> {
    use crate::remote::{self, commands};
    let mut results = SearchResults::default();
    if query.text.trim().is_empty() {
        return Ok(results);
    }
    let remote_roots: Vec<(PathBuf, remote::RemotePath)> =
        roots.iter().filter_map(|root| Some((root.clone(), remote::from_mirror(root)?))).collect();
    let mut hosts: Vec<&str> = Vec::new();
    for (_, found) in &remote_roots {
        if !hosts.contains(&found.host.as_str()) {
            hosts.push(&found.host);
        }
    }
    for host in hosts {
        let folders: Vec<&(PathBuf, remote::RemotePath)> =
            remote_roots.iter().filter(|(_, found)| found.host == host).collect();
        let remote_folders: Vec<PathBuf> = folders.iter().map(|(_, found)| found.path.clone()).collect();
        // The host finds the files containing the query without case; the
        // options are applied here, on their text.
        let found = commands::search(host, &remote_folders, query.text, MAX_FILE_BYTES)?;
        let mut matching = found.text_matches;
        matching.truncate(MAX_REMOTE_TEXT_FILES);
        let texts = commands::read_many(host, &matching, MAX_FILE_BYTES)?;
        let mut files = found.files;
        files.sort_by_key(|file| file.to_string_lossy().to_lowercase());
        files.dedup();
        for file in files {
            if results.truncated {
                return Ok(results);
            }
            // The longest folder holding the file is its explorer root.
            let Some((root, _)) = folders
                .iter()
                .filter(|(_, folder)| file.starts_with(&folder.path))
                .max_by_key(|(_, folder)| folder.path.as_os_str().len())
            else {
                continue;
            };
            let Some(mirror) = remote::to_mirror(host, &file) else {
                continue;
            };
            let text = matching
                .iter()
                .position(|candidate| *candidate == file)
                .and_then(|index| texts.get(index).cloned().flatten())
                .and_then(|bytes| String::from_utf8(bytes).ok());
            search_file(&mirror, root, text.as_deref(), query, limit, &mut results);
        }
    }
    Ok(results)
}

fn push_hit(results: &mut SearchResults, hit: FileHit, limit: usize) {
    if results.hits.len() >= limit {
        results.truncated = true;
    } else {
        results.hits.push(hit);
    }
}

/// The 1-based line number of `range` (a match in `text`), its line as an
/// excerpt, and the match's byte range in that excerpt.
///
/// The excerpt is the trimmed line cut to [`EXCERPT_CHARS`]. When the match
/// would fall past that cut, the excerpt starts [`EXCERPT_CONTEXT_CHARS`]
/// before the match instead, so the match is always shown with some text
/// around it.
fn excerpt_of(text: &str, range: &Range<usize>) -> (usize, String, Range<usize>) {
    let at = range.start;
    let line_start = text[..at].rfind('\n').map_or(0, |index| index + 1);
    let line_end = text[at..].find('\n').map_or(text.len(), |index| at + index);
    let line_number = text[..at].matches('\n').count() + 1;
    let line = &text[line_start..line_end];
    let trimmed_start = line_start + (line.len() - line.trim_start().len());
    let trimmed_end = (line_start + line.trim_end().len()).max(trimmed_start);
    // The part of the match inside the trimmed line (a match on the spaces
    // at the edge of a line has none, and gets an empty highlight).
    let match_start = at.clamp(trimmed_start, trimmed_end);
    let match_end = range.end.clamp(match_start, trimmed_end);

    let chars_to_match_end = text[trimmed_start..match_end].chars().count();
    let excerpt_start = if chars_to_match_end <= EXCERPT_CHARS {
        trimmed_start
    } else {
        let before_match = &text[trimmed_start..match_start];
        let skip = before_match.chars().count().saturating_sub(EXCERPT_CONTEXT_CHARS);
        before_match.char_indices().nth(skip).map_or(match_start, |(offset, _)| trimmed_start + offset)
    };
    let mut excerpt = String::new();
    if excerpt_start > trimmed_start {
        excerpt.push(ELLIPSIS);
    }
    let prefix_len = excerpt.len();
    excerpt.extend(text[excerpt_start..trimmed_end].chars().take(EXCERPT_CHARS));
    let kept_end = excerpt_start + (excerpt.len() - prefix_len);
    if kept_end < trimmed_end {
        excerpt.push(ELLIPSIS);
    }
    // A match longer than the excerpt is highlighted up to the cut.
    let highlight_start = prefix_len + (match_start - excerpt_start);
    let highlight_end = prefix_len + (match_end.min(kept_end) - excerpt_start);
    (line_number, excerpt, highlight_start..highlight_end.max(highlight_start))
}

/// The character index of byte `at` in `text`.
pub fn char_index(text: &str, at: usize) -> usize {
    text[..at.min(text.len())].chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempFolder;

    #[test]
    fn remote_search_finds_names_and_text_like_local_search() {
        let temp = TempFolder::new();
        temp.write("Plan.md", "nothing");
        temp.write("docs/b.md", "line one\nthe PLAN is here\n");
        temp.write(".git/plan", "plan");
        let mirror = crate::remote::testing::mirror_of("find-host", temp.path());
        let results = search_remote_files(std::slice::from_ref(&mirror), plain("plan"), 50).unwrap();
        assert_eq!(results.hits.len(), 2);
        // Same order as a local search: `docs` sorts before `Plan.md`.
        assert_eq!(
            results.hits[1],
            FileHit { path: mirror.join("Plan.md"), root: mirror.clone(), kind: HitKind::Name }
        );
        let HitKind::Text { line, excerpt, highlight, range } = &results.hits[0].kind else {
            panic!("text hit expected");
        };
        assert_eq!(results.hits[0].path, mirror.join("docs/b.md"));
        assert_eq!((*line, excerpt.as_str(), highlight.clone(), range.clone()), (2, "the PLAN is here", 4..8, 13..17));
    }

    /// A query with the default options: without case, not whole words.
    fn plain(text: &str) -> Query<'_> {
        Query { text, options: SearchOptions::default() }
    }

    const MATCH_CASE: SearchOptions = SearchOptions { match_case: true, whole_word: false };
    const WHOLE_WORD: SearchOptions = SearchOptions { match_case: false, whole_word: true };

    #[test]
    fn finds_without_case_and_without_overlaps() {
        let any = SearchOptions::default();
        assert_eq!(find_all("Ab ab AB", "ab", any), vec![0..2, 3..5, 6..8]);
        assert_eq!(find_all("aaaa", "aa", any), vec![0..2, 2..4]);
        assert_eq!(find_all("text", "", any), Vec::<Range<usize>>::new());
        assert_eq!(find_all("", "x", any), Vec::<Range<usize>>::new());
    }

    #[test]
    fn match_case_keeps_only_the_same_case() {
        assert_eq!(find_all("Ab ab AB", "ab", MATCH_CASE), vec![3..5]);
        assert_eq!(find_all("Київ київ", "київ", MATCH_CASE), vec![9..17]);
        assert_eq!(find_all("Київ київ", "київ", SearchOptions::default()), vec![0..8, 9..17]);
    }

    #[test]
    fn whole_word_rejects_matches_inside_words() {
        let text = "cat concat cat_ (cat) Cat.";
        assert_eq!(find_all(text, "cat", WHOLE_WORD), vec![0..3, 17..20, 22..25]);
        let both = SearchOptions { match_case: true, whole_word: true };
        assert_eq!(find_all(text, "cat", both), vec![0..3, 17..20]);
        // Letters outside ASCII are word characters too.
        assert_eq!(find_all("étécode été", "été", WHOLE_WORD), vec![10..15]);
        assert_eq!(find_all("aaa", "aa", WHOLE_WORD), Vec::<Range<usize>>::new());
    }

    #[test]
    fn lower_casing_keeps_original_offsets() {
        // İ (2 bytes) lower-cases to i̇ (3 bytes); the match after it still
        // points into the original text.
        let any = SearchOptions::default();
        let text = "İstanbul ÉTÉ";
        let ranges = find_all(text, "été", any);
        assert_eq!(ranges.len(), 1);
        assert_eq!(&text[ranges[0].clone()], "ÉTÉ");
        assert_eq!(&text[find_all(text, "i̇stan", any)[0].clone()], "İstan");
    }

    #[test]
    fn replaces_every_match_with_shorter_longer_or_empty_text() {
        let text = "one two one";
        let matches = find_all(text, "one", SearchOptions::default());
        assert_eq!(replace_all(text, &matches, "1"), "1 two 1");
        assert_eq!(replace_all(text, &matches, "three"), "three two three");
        assert_eq!(replace_all(text, &matches, ""), " two ");
        assert_eq!(replace_all(text, &[], "x"), text);
        // The replacement may contain the query; it is not matched again.
        assert_eq!(replace_all(text, &matches, "one one"), "one one two one one");
    }

    #[test]
    fn line_numbers_and_excerpts() {
        let text = "first\n  second line here \nthird";
        assert_eq!(excerpt_of(text, &(0..5)), (1, "first".to_string(), 0..5));
        assert_eq!(excerpt_of(text, &(15..19)), (2, "second line here".to_string(), 7..11));
        assert_eq!(excerpt_of(text, &(26..31)), (3, "third".to_string(), 0..5));
        // A match on the spaces at the edge of a line gets an empty highlight.
        assert_eq!(excerpt_of(text, &(6..8)), (2, "second line here".to_string(), 0..0));
        assert_eq!(char_index("héllo", 3), 2);
    }

    #[test]
    fn long_lines_keep_the_match_in_the_excerpt() {
        // Near the start: the line is cut at the end only.
        let long = format!("milk {}", "é".repeat(200));
        let (_, excerpt, highlight) = excerpt_of(&long, &(0..4));
        assert_eq!(excerpt.chars().count(), EXCERPT_CHARS + 1);
        assert!(excerpt.ends_with(ELLIPSIS));
        assert_eq!(&excerpt[highlight], "milk");

        // Far along: the excerpt starts a little before the match.
        let text = format!("{}milk tail", "é".repeat(150));
        let start = text.find("milk").unwrap();
        let (_, excerpt, highlight) = excerpt_of(&text, &(start..start + 4));
        assert!(excerpt.starts_with(ELLIPSIS));
        assert!(!excerpt.ends_with(ELLIPSIS));
        assert_eq!(&excerpt[highlight.clone()], "milk");
        assert_eq!(excerpt[..highlight.start].chars().count(), EXCERPT_CONTEXT_CHARS + 1);
    }

    #[test]
    fn searches_files_by_name_and_text() {
        let folder = TempFolder::new();
        folder.write("notes/Plan.md", "# Plan\nbuy milk\nMilk is white\n");
        folder.write("notes/.hidden.md", "milk\n");
        folder.write("other.txt", "no match\n");
        folder.write("image.png", "\0\0milk");
        let roots = vec![folder.path().to_path_buf()];
        let results = search_files(&roots, plain("milk"), 10, &AtomicBool::new(false));
        let kinds: Vec<&HitKind> = results.hits.iter().map(|hit| &hit.kind).collect();
        assert_eq!(
            kinds,
            vec![
                &HitKind::Text { line: 2, excerpt: "buy milk".into(), highlight: 4..8, range: 11..15 },
                &HitKind::Text { line: 3, excerpt: "Milk is white".into(), highlight: 0..4, range: 16..20 },
            ]
        );
        assert_eq!(results.hits[0].path, folder.path().join("notes/Plan.md"));
        assert_eq!(results.hits[0].root, folder.path());
        assert_eq!(results.file_count(), 1);
        assert_eq!(results.files_searched, 2);

        let by_name = search_files(&roots, plain("plan"), 10, &AtomicBool::new(false));
        assert_eq!(by_name.hits[0].kind, HitKind::Name);
        assert_eq!(
            by_name.hits[1].kind,
            HitKind::Text { line: 1, excerpt: "# Plan".into(), highlight: 2..6, range: 2..6 }
        );

        let capped = search_files(&roots, plain("milk"), 1, &AtomicBool::new(false));
        assert_eq!(capped.hits.len(), 1);
        assert!(capped.truncated);
        assert_eq!(search_files(&roots, plain("  "), 10, &AtomicBool::new(false)).hits, Vec::<FileHit>::new());

        // The options apply to the text of the files; names ignore the case.
        let exact = search_files(&roots, Query { text: "Milk", options: MATCH_CASE }, 10, &AtomicBool::new(false));
        assert_eq!(exact.hits.len(), 1);
        assert_eq!(
            exact.hits[0].kind,
            HitKind::Text { line: 3, excerpt: "Milk is white".into(), highlight: 0..4, range: 16..20 }
        );
        let by_name = search_files(&roots, Query { text: "PLAN", options: MATCH_CASE }, 10, &AtomicBool::new(false));
        assert_eq!(
            by_name.hits,
            vec![FileHit {
                path: folder.path().join("notes/Plan.md"),
                root: folder.path().to_path_buf(),
                kind: HitKind::Name
            }]
        );
    }

    #[test]
    fn a_cancelled_search_stops_before_reading_files() {
        let folder = TempFolder::new();
        folder.write("milk.md", "milk\n");
        folder.write("notes/b.md", "more milk\n");
        let results = search_files(&[folder.path().to_path_buf()], plain("milk"), 10, &AtomicBool::new(true));
        assert_eq!(results, SearchResults::default());
    }

    #[test]
    fn skips_the_text_of_encrypted_scratches() {
        let folder = TempFolder::new();
        // Real ones are rarely valid UTF-8; this one is, to show the check.
        folder.write("secret-milk.md", "RUBRIQUE-ENC1\nmilk");
        let results = search_files(&[folder.path().to_path_buf()], plain("milk"), 10, &AtomicBool::new(false));
        assert_eq!(results.hits.len(), 1);
        assert_eq!(results.hits[0].kind, HitKind::Name);
    }
}
