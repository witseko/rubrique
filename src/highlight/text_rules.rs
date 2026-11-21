//! Text highlight rules: regular expressions from Settings that color
//! matches (file paths, IP addresses, numbers, UUIDs, hashes) in plain text files.
//!
//! Rules are tried top to bottom and the first rule that matches a piece of
//! text colors it, so a specific rule (a UUID) wins over a broad one (a
//! number) placed below it. A rule whose pattern does not compile is
//! skipped; [`pattern_error`] says why, for Settings to show.

use std::collections::BTreeMap;
use std::ops::Range;

use regex::{Regex, RegexBuilder};

use crate::theme::palette::Rgb;

/// Largest compiled program one pattern may grow to. Patterns such as
/// `\w{1000}{1000}` would otherwise take a lot of memory and time to build.
const PATTERN_SIZE_LIMIT: usize = 1 << 20;
/// Longer texts are left plain: every rule scans the whole text again after
/// each edit, and a huge log should stay quick to type in.
pub const MAX_HIGHLIGHT_BYTES: usize = 2_000_000;

/// The color of a rule: one of the code colors (so it follows the theme and
/// the Code colors setting) or a color the user picked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RuleColor {
    Keyword,
    String,
    Number,
    Name,
    Key,
    Comment,
    Custom(Rgb),
}

impl RuleColor {
    /// The code colors a rule can borrow, in the order Settings offers them.
    pub const PALETTE: [RuleColor; 6] =
        [RuleColor::Keyword, RuleColor::String, RuleColor::Number, RuleColor::Name, RuleColor::Key, RuleColor::Comment];

    /// The word stored in the settings file: a palette name or `#rrggbb`.
    pub fn id(self) -> String {
        match self {
            RuleColor::Keyword => "keyword".to_string(),
            RuleColor::String => "string".to_string(),
            RuleColor::Number => "number".to_string(),
            RuleColor::Name => "name".to_string(),
            RuleColor::Key => "key".to_string(),
            RuleColor::Comment => "comment".to_string(),
            RuleColor::Custom(Rgb(red, green, blue)) => format!("#{red:02x}{green:02x}{blue:02x}"),
        }
    }

    /// Read a stored word; `None` when it is neither a palette name nor a color.
    pub fn parse(value: &str) -> Option<Self> {
        RuleColor::PALETTE.into_iter().find(|color| color.id() == value).or_else(|| {
            // Only `#rrggbb` is a custom color; a bare word is a palette name.
            value.starts_with('#').then(|| Rgb::parse(value)).flatten().map(RuleColor::Custom)
        })
    }

    /// Shown in the color picker.
    pub fn name(self) -> &'static str {
        match self {
            RuleColor::Keyword => "Keyword",
            RuleColor::String => "String",
            RuleColor::Number => "Number",
            RuleColor::Name => "Name",
            RuleColor::Key => "Key",
            RuleColor::Comment => "Comment",
            RuleColor::Custom(_) => "Custom",
        }
    }
}

/// One rule as the user sees and edits it in Settings.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct HighlightRule {
    pub name: String,
    /// A regular expression in the `regex` crate's syntax.
    pub pattern: String,
    pub color: RuleColor,
    pub enabled: bool,
}

impl HighlightRule {
    /// A switched-on rule.
    pub fn new(name: &str, pattern: &str, color: RuleColor) -> Self {
        HighlightRule { name: name.to_string(), pattern: pattern.to_string(), color, enabled: true }
    }

    /// The settings value: `on`/`off`, color, pattern and name, joined by
    /// tabs. The name goes last so the settings reader, which trims the
    /// value, can only cut spaces from the end of the name, never from the
    /// pattern. Tabs and line breaks inside the pattern are written as
    /// `\t` and `\n`, which mean the same in a regular expression.
    pub fn to_value(&self) -> String {
        let pattern = self.pattern.replace('\t', "\\t").replace('\n', "\\n");
        let name = self.name.replace(['\t', '\n'], " ");
        let state = if self.enabled { "on" } else { "off" };
        format!("{state}\t{}\t{pattern}\t{name}", self.color.id())
    }

    /// Read a value written by [`HighlightRule::to_value`]; `None` when it is broken.
    pub fn from_value(value: &str) -> Option<Self> {
        let mut parts = value.splitn(4, '\t');
        let enabled = match parts.next()? {
            "on" => true,
            "off" => false,
            _ => return None,
        };
        let color = RuleColor::parse(parts.next()?)?;
        let pattern = parts.next()?.to_string();
        let name = parts.next().unwrap_or_default().trim().to_string();
        Some(HighlightRule { name, pattern, color, enabled })
    }
}

/// Octet of an IPv4 address: 0 to 255, without leading zeros.
const OCTET: &str = "(?:25[0-5]|2[0-4][0-9]|1[0-9]{2}|[1-9]?[0-9])";

/// One part of a file path, between two separators.
const PATH_PART: &str = r"[\w.@+-]+";

/// Extensions the File name rule knows. A name needs one of them, so
/// `example.com`, `e.g` and `com.example.Foo` stay plain.
const FILE_EXTENSIONS: &str = "txt|md|markdown|rst|log|csv|tsv|pdf|docx?|xlsx?|pptx?|rtf|tex|\
    json|jsonl|ya?ml|toml|xml|ini|cfg|conf|env|lock|sql|sqlite|plist|\
    html?|css|scss|m?js|cjs|jsx|tsx?|vue|svelte|\
    rs|py|rb|go|java|kt|swift|c|h|cpp|hpp|cc|cs|php|sh|bash|zsh|ps1|bat|lua|\
    png|jpe?g|gif|svg|webp|avif|ico|bmp|tiff?|heic|mp3|mp4|wav|mov|mkv|flac|ogg|webm|\
    zip|tar|gz|tgz|bz2|xz|7z|rar|dmg|iso|deb|rpm|pkg|exe|dll|so|dylib|jar|\
    bak|tmp|patch|diff|pem|crt";

/// The File path pattern: paths with a separator, in three forms.
fn file_path_pattern() -> String {
    // `~/notes`, `./a`, `../a` and `/usr/bin`. A path from the root needs two
    // parts, so `</div>` and `/help` stay plain.
    let rooted = format!(r"\B(?:(?:~|\.\.?)/{PATH_PART}|/{PATH_PART}/{PATH_PART})(?:/{PATH_PART})*\b/?");
    let windows = format!(r"\b[A-Za-z]:\\{PATH_PART}(?:\\{PATH_PART})*\b\\?");
    // `src/app/main.rs`. It must end in an extension, so `and/or`,
    // `11/21/2026` and `application/json` stay plain.
    let relative = format!(r"\b\w[\w.@+-]*(?:/{PATH_PART})*/[\w.@+-]*\.[A-Za-z][A-Za-z0-9]*\b");
    format!("{rooted}|{windows}|{relative}")
}

/// The rules a new install starts with. They are ordinary rules: the user can
/// change, reorder, switch off or delete each one.
pub fn default_rules() -> Vec<HighlightRule> {
    let ipv4 = format!(r"\b(?:{OCTET}\.){{3}}{OCTET}\b");
    let file_name = format!(r"(?i)\b\w[\w.-]*\.(?:{FILE_EXTENSIONS})\b");
    vec![
        // URL and File path go first: they hold dates, numbers, addresses and
        // ids, and a rule above them would take those pieces and leave the
        // rest plain. URL is above File path so a link's path stays its own.
        HighlightRule::new("URL", r#"\b[A-Za-z][A-Za-z0-9+.-]*://[^\s<>"']*[\w/]"#, RuleColor::String),
        HighlightRule::new("File path", &file_path_pattern(), RuleColor::Name),
        HighlightRule::new(
            "UUID",
            r"(?i)\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b",
            RuleColor::Name,
        ),
        HighlightRule::new("IPv4 address", &ipv4, RuleColor::Keyword),
        // Full form, then the shortened forms with `::` (fe80::1, ::1,
        // 2001:db8::). Clock times like 12:30:45 have neither.
        HighlightRule::new(
            "IPv6 address",
            r"(?i)\b(?:[0-9a-f]{1,4}:){7}[0-9a-f]{1,4}\b|\b(?:[0-9a-f]{1,4}:){1,6}(?::[0-9a-f]{1,4}){1,6}\b|\b(?:[0-9a-f]{1,4}:){1,7}:|::(?:[0-9a-f]{1,4}:){0,5}[0-9a-f]{1,4}\b",
            RuleColor::Keyword,
        ),
        HighlightRule::new("Email", r"\b[\w.+-]+@[\w-]+(?:\.[\w-]+)+\b", RuleColor::String),
        // A name without a folder (`notes.txt`). Below Email, whose host
        // can end like a file (`ops@example.sh`).
        HighlightRule::new("File name", &file_name, RuleColor::Name),
        HighlightRule::new("Date", r"\b[0-9]{4}-[0-9]{2}-[0-9]{2}\b", RuleColor::Key),
        // Whole words only, so the digits inside a hash or an id stay theirs.
        HighlightRule::new("Number", r"\b[0-9]+(?:\.[0-9]+)?\b", RuleColor::Number),
        // Git commit ids (short and full) and SHA-256 sums. Below Number, so
        // a long plain number stays a number.
        HighlightRule::new("Hash", r"\b[0-9a-f]{7,64}\b", RuleColor::Comment),
    ]
}

/// Number of the current set of default rules. Raise it when a release adds
/// default rules, and name them in [`add_new_default_rules`].
pub const DEFAULT_RULES_VERSION: u32 = 2;
/// The default rules added in version 2.
const ADDED_IN_VERSION_2: [&str; 3] = ["URL", "File path", "File name"];

/// Add the default rules that came after `seen_version` to a saved list.
/// Each goes right under the nearest default rule above it that the list
/// still holds (else on top), so it keeps its place in the order. A rule
/// whose name or pattern the list already has is left out, and an empty
/// list stays empty: its user deleted every rule.
pub fn add_new_default_rules(rules: &mut Vec<HighlightRule>, seen_version: u32) {
    if seen_version >= DEFAULT_RULES_VERSION || rules.is_empty() {
        return;
    }
    let defaults = default_rules();
    for (place, new_rule) in defaults.iter().enumerate() {
        let is_new = ADDED_IN_VERSION_2.contains(&new_rule.name.as_str());
        if !is_new || rules.iter().any(|rule| is_same_rule(rule, new_rule)) {
            continue;
        }
        let under = defaults[..place]
            .iter()
            .rev()
            .find_map(|above| rules.iter().position(|rule| rule.pattern == above.pattern));
        rules.insert(under.map_or(0, |position| position + 1), new_rule.clone());
    }
}

/// Whether two rules have the same pattern or the same name (any letter case).
fn is_same_rule(left: &HighlightRule, right: &HighlightRule) -> bool {
    left.pattern == right.pattern || left.name.eq_ignore_ascii_case(&right.name)
}

/// Compile one pattern with Rubrique's limits. Matching is linear in the
/// text's length for every pattern (the `regex` crate has no backtracking).
fn compile(pattern: &str) -> Result<Regex, String> {
    if pattern.is_empty() {
        return Err("Write a pattern".to_string());
    }
    RegexBuilder::new(pattern).size_limit(PATTERN_SIZE_LIMIT).build().map_err(|error| short_error(&error))
}

/// The last line of the crate's message ("unclosed group"), without the
/// drawing of the pattern above it.
fn short_error(error: &regex::Error) -> String {
    let text = error.to_string();
    let last = text.lines().rev().map(str::trim).find(|line| !line.is_empty()).unwrap_or("Invalid pattern");
    last.strip_prefix("error: ").unwrap_or(last).to_string()
}

/// Why `pattern` cannot be used, or `None` when it compiles.
pub fn pattern_error(pattern: &str) -> Option<String> {
    compile(pattern).err()
}

/// The switched-on rules that compile, built once when the rules change and
/// shared by every layout until they change again. Two are equal (and hash
/// the same) when they come from the same rules, so layouts cached for one
/// stay valid for the other.
#[derive(Debug, Default)]
pub struct CompiledRules {
    /// The rules as written, to tell when they changed.
    source: Vec<HighlightRule>,
    /// The usable rules, in order, each with its place in `source`.
    compiled: Vec<(usize, Regex)>,
}

impl CompiledRules {
    /// Compile the switched-on rules; broken patterns are left out.
    pub fn new(rules: &[HighlightRule]) -> Self {
        let compiled = rules
            .iter()
            .enumerate()
            .filter(|(_, rule)| rule.enabled)
            .filter_map(|(index, rule)| Some((index, compile(&rule.pattern).ok()?)))
            .collect();
        CompiledRules { source: rules.to_vec(), compiled }
    }

    /// The rules these were compiled from.
    pub fn source(&self) -> &[HighlightRule] {
        &self.source
    }

    /// The colored pieces of `text`, in order and never overlapping. Each
    /// piece belongs to the first rule that matched it; a match that would
    /// overlap a piece already taken is dropped.
    pub fn spans(&self, text: &str) -> Vec<(Range<usize>, RuleColor)> {
        self.pieces(text).into_iter().map(|(range, index)| (range, self.source[index].color)).collect()
    }

    /// How many pieces of `text` each rule colors, by its place in the rules
    /// (zero for rules that are off or broken), for Settings' sample.
    pub fn match_counts(&self, text: &str) -> Vec<usize> {
        let mut counts = vec![0; self.source.len()];
        for (_, index) in self.pieces(text) {
            counts[index] += 1;
        }
        counts
    }

    /// The pieces [`CompiledRules::spans`] colors, each with the place of the
    /// rule that took it.
    fn pieces(&self, text: &str) -> Vec<(Range<usize>, usize)> {
        if text.len() > MAX_HIGHLIGHT_BYTES {
            return Vec::new();
        }
        // Start of each piece taken so far, with its end and rule.
        let mut taken: BTreeMap<usize, (usize, usize)> = BTreeMap::new();
        for (index, regex) in &self.compiled {
            for found in regex.find_iter(text) {
                let range = found.range();
                if !range.is_empty() && !overlaps_taken(&taken, &range) {
                    taken.insert(range.start, (range.end, *index));
                }
            }
        }
        taken.into_iter().map(|(start, (end, index))| (start..end, index)).collect()
    }
}

impl PartialEq for CompiledRules {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
    }
}

impl Eq for CompiledRules {}

impl std::hash::Hash for CompiledRules {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.source.hash(state);
    }
}

/// Whether `range` overlaps a taken piece. Pieces never overlap each other,
/// so only the last one starting before `range` ends can reach into it.
fn overlaps_taken(taken: &BTreeMap<usize, (usize, usize)>, range: &Range<usize>) -> bool {
    taken.range(..range.end).next_back().is_some_and(|(_, (end, _))| *end > range.start)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The text of each colored piece, with its color.
    fn pieces<'a>(rules: &CompiledRules, text: &'a str) -> Vec<(&'a str, RuleColor)> {
        rules.spans(text).into_iter().map(|(range, color)| (&text[range], color)).collect()
    }

    /// The matched text of the default rule called `name` in `text`.
    fn default_matches<'a>(name: &str, text: &'a str) -> Vec<&'a str> {
        let rule = default_rules().into_iter().find(|rule| rule.name == name).expect("a default rule");
        pieces(&CompiledRules::new(&[rule]), text).into_iter().map(|(piece, _)| piece).collect()
    }

    #[test]
    fn every_default_rule_compiles() {
        for rule in default_rules() {
            assert_eq!(pattern_error(&rule.pattern), None, "{}", rule.name);
        }
    }

    #[test]
    fn default_rules_find_what_they_say() {
        assert_eq!(default_matches("IPv4 address", "from 10.0.0.255 to 256.1.1.1"), ["10.0.0.255"]);
        assert_eq!(
            default_matches("IPv6 address", "fe80::1 and 2001:db8::8a2e:370:7334"),
            ["fe80::1", "2001:db8::8a2e:370:7334"]
        );
        assert_eq!(
            default_matches("IPv6 address", "::1 2001:0db8:0000:0000:0000:ff00:0042:8329"),
            ["::1", "2001:0db8:0000:0000:0000:ff00:0042:8329"]
        );
        assert_eq!(default_matches("IPv6 address", "at 12:30:45 sharp"), Vec::<&str>::new());
        assert_eq!(
            default_matches("UUID", "id 550E8400-e29b-41d4-a716-446655440000."),
            ["550E8400-e29b-41d4-a716-446655440000"]
        );
        assert_eq!(default_matches("Hash", "commit 3f9a2c1 and deadbeefcafe0"), ["3f9a2c1", "deadbeefcafe0"]);
        assert_eq!(default_matches("Hash", "abc12 is too short"), Vec::<&str>::new());
        assert_eq!(default_matches("Number", "v2 has 42 and 3.14"), ["42", "3.14"]);
        assert_eq!(default_matches("Date", "due 2026-11-10."), ["2026-11-10"]);
        assert_eq!(default_matches("Email", "write to a.b+c@mail.example.org."), ["a.b+c@mail.example.org"]);
    }

    #[test]
    fn file_path_rule_finds_paths() {
        assert_eq!(
            default_matches("File path", "open /usr/local/bin, ~/notes/todo.txt or ../docs/."),
            ["/usr/local/bin", "~/notes/todo.txt", "../docs/"]
        );
        assert_eq!(
            default_matches("File path", r"at src/app/main.rs:42 and C:\Users\me\notes.txt."),
            ["src/app/main.rs", r"C:\Users\me\notes.txt"]
        );
        assert_eq!(default_matches("File path", "PATH=/usr/bin:/opt/tools/bin"), ["/usr/bin", "/opt/tools/bin"]);
        assert_eq!(
            default_matches("File path", "see node_modules/@types/node/index.d.ts"),
            ["node_modules/@types/node/index.d.ts"]
        );
    }

    #[test]
    fn file_path_rule_leaves_other_slashes_plain() {
        let plain = "and/or 11/21/2026 10.0.0.0/24 application/json </div> /help 1/2 // note";
        assert_eq!(default_matches("File path", plain), Vec::<&str>::new());
    }

    #[test]
    fn file_name_rule_finds_names_with_a_known_extension() {
        assert_eq!(
            default_matches("File name", "see notes.txt, Cargo.TOML and my-archive.tar.gz."),
            ["notes.txt", "Cargo.TOML", "my-archive.tar.gz"]
        );
        assert_eq!(default_matches("File name", "--config=app.yaml"), ["app.yaml"]);
        let plain = "e.g. example.com, com.example.Foo, 9 a.m. and v1.2";
        assert_eq!(default_matches("File name", plain), Vec::<&str>::new());
    }

    #[test]
    fn url_rule_finds_links_without_the_closing_punctuation() {
        assert_eq!(
            default_matches("URL", "see https://example.com/a/b.html, or (file:///tmp/x)."),
            ["https://example.com/a/b.html", "file:///tmp/x"]
        );
        assert_eq!(default_matches("URL", "ratio 3://4"), Vec::<&str>::new());
    }

    #[test]
    fn urls_and_paths_keep_the_pieces_inside_them() {
        let rules = CompiledRules::new(&default_rules());
        let text = "GET http://10.0.0.1:8080/v2/a.json saved /var/log/2026-11-10/app.log by ops@example.sh as out.csv";
        assert_eq!(
            pieces(&rules, text),
            [
                ("http://10.0.0.1:8080/v2/a.json", RuleColor::String),
                ("/var/log/2026-11-10/app.log", RuleColor::Name),
                ("ops@example.sh", RuleColor::String),
                ("out.csv", RuleColor::Name),
            ]
        );
    }

    /// The default rules as they were before version 2.
    fn version_1_rules() -> Vec<HighlightRule> {
        default_rules().into_iter().filter(|rule| !ADDED_IN_VERSION_2.contains(&rule.name.as_str())).collect()
    }

    #[test]
    fn new_defaults_take_their_place_in_an_old_list() {
        let mut rules = version_1_rules();
        add_new_default_rules(&mut rules, 1);
        assert_eq!(rules, default_rules());
    }

    #[test]
    fn new_defaults_keep_the_users_own_rules_and_order() {
        let ticket = HighlightRule::new("Ticket", "PROJ-[0-9]+", RuleColor::Key);
        let own_url = HighlightRule::new("url", "https?://[a-z./]+", RuleColor::Comment);
        let email = default_rules().into_iter().find(|rule| rule.name == "Email").expect("a default rule");
        let mut rules = vec![ticket.clone(), own_url.clone(), email.clone()];
        add_new_default_rules(&mut rules, 1);
        let names: Vec<&str> = rules.iter().map(|rule| rule.name.as_str()).collect();
        // The user's own "url" rule stands in for the default one.
        assert_eq!(names, ["File path", "Ticket", "url", "Email", "File name"]);
    }

    #[test]
    fn new_defaults_skip_current_and_empty_lists() {
        let mut current = version_1_rules();
        add_new_default_rules(&mut current, DEFAULT_RULES_VERSION);
        assert_eq!(current, version_1_rules());
        let mut empty = Vec::new();
        add_new_default_rules(&mut empty, 1);
        assert_eq!(empty, Vec::new());
    }

    #[test]
    fn the_first_rule_wins_an_overlap() {
        let rules = CompiledRules::new(&default_rules());
        let text = "host 192.168.1.10 id 550e8400-e29b-41d4-a716-446655440000 count 7";
        assert_eq!(
            pieces(&rules, text),
            [
                ("192.168.1.10", RuleColor::Keyword),
                ("550e8400-e29b-41d4-a716-446655440000", RuleColor::Name),
                ("7", RuleColor::Number),
            ]
        );
    }

    #[test]
    fn rule_order_decides() {
        let number = HighlightRule::new("Number", "[0-9]+", RuleColor::Number);
        let year = HighlightRule::new("Year", "20[0-9]{2}", RuleColor::Key);
        let number_first = CompiledRules::new(&[number.clone(), year.clone()]);
        assert_eq!(pieces(&number_first, "in 2026"), [("2026", RuleColor::Number)]);
        let year_first = CompiledRules::new(&[year, number]);
        assert_eq!(pieces(&year_first, "in 2026"), [("2026", RuleColor::Key)]);
    }

    #[test]
    fn broken_switched_off_and_empty_rules_are_skipped() {
        let rules = CompiledRules::new(&[
            HighlightRule::new("Broken", "(unclosed", RuleColor::Keyword),
            HighlightRule { enabled: false, ..HighlightRule::new("Off", "[a-z]+", RuleColor::Name) },
            HighlightRule::new("Empty matches", "x*", RuleColor::Key),
            HighlightRule::new("Digits", "[0-9]+", RuleColor::Number),
        ]);
        assert_eq!(pieces(&rules, "abc 12"), [("12", RuleColor::Number)]);
    }

    #[test]
    fn match_counts_follow_the_rule_that_wins() {
        let rules = CompiledRules::new(&[
            HighlightRule::new("Year", "20[0-9]{2}", RuleColor::Key),
            HighlightRule { enabled: false, ..HighlightRule::new("Off", "[a-z]+", RuleColor::Name) },
            HighlightRule::new("Number", "[0-9]+", RuleColor::Number),
        ]);
        // 2026 goes to Year, so Number only counts 7 and 12.
        assert_eq!(rules.match_counts("in 2026, 7 of 12"), [1, 0, 2]);
    }

    #[test]
    fn pattern_errors_are_short() {
        assert_eq!(pattern_error("[0-9]+"), None);
        assert_eq!(pattern_error("").as_deref(), Some("Write a pattern"));
        let error = pattern_error("(unclosed").expect("an error");
        assert!(!error.contains('\n'), "{error}");
        assert!(error.contains("unclosed"), "{error}");
        // Look-around is not supported by the crate and is reported, not a crash.
        assert!(pattern_error("(?=a)b").is_some());
        // A pattern that would compile to a huge program is refused.
        assert!(pattern_error(r"\w{1000}{1000}").is_some());
    }

    #[test]
    fn huge_texts_stay_plain() {
        let rules = CompiledRules::new(&default_rules());
        let text = "1 ".repeat(MAX_HIGHLIGHT_BYTES / 2 + 1);
        assert_eq!(rules.spans(&text), Vec::new());
    }

    #[test]
    fn rules_round_trip_through_values() {
        let rules = [
            HighlightRule::new("Ticket", r"\bPROJ-[0-9]+\b", RuleColor::Custom(Rgb(0xff, 0x80, 0x00))),
            HighlightRule { enabled: false, ..HighlightRule::new("Spaces ", "a b  ", RuleColor::Comment) },
        ];
        for rule in rules {
            let parsed = HighlightRule::from_value(rule.to_value().trim()).expect("parses");
            assert_eq!(parsed, HighlightRule { name: rule.name.trim().to_string(), ..rule });
        }
    }

    #[test]
    fn tabs_in_patterns_are_written_as_escapes() {
        let rule = HighlightRule::new("Tab\tname", "a\tb", RuleColor::Key);
        let parsed = HighlightRule::from_value(&rule.to_value()).expect("parses");
        assert_eq!(parsed.pattern, r"a\tb");
        assert_eq!(parsed.name, "Tab name");
    }

    #[test]
    fn broken_values_are_ignored() {
        assert_eq!(HighlightRule::from_value("maybe\tkeyword\t[0-9]\tX"), None);
        assert_eq!(HighlightRule::from_value("on\tpurple\t[0-9]\tX"), None);
        assert_eq!(HighlightRule::from_value("on"), None);
        assert_eq!(RuleColor::parse("#12345"), None);
        assert_eq!(RuleColor::parse("#a0b1c2"), Some(RuleColor::Custom(Rgb(0xa0, 0xb1, 0xc2))));
    }
}
