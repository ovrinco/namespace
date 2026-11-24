use crate::{Result, manifest::Namespace, read};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};
#[derive(Deserialize)]
pub struct Names {
    #[serde(default)]
    pub name: Vec<Name>,
    #[serde(default)]
    pub retired: Vec<Retired>,
}
#[derive(Deserialize)]
pub struct Name {
    pub form: String,
    pub wrong: Vec<String>,
    /// Wrong variants that are also ordinary words or keys, with the use that is not a name.
    #[serde(default)]
    pub ordinary: Vec<Ordinary>,
}
#[derive(Deserialize)]
pub struct Ordinary {
    pub wrong: String,
    #[serde(rename = "use")]
    pub use_: String,
}
#[derive(Deserialize)]
pub struct Retired {
    pub name: String,
    pub current: Option<String>,
}
#[derive(Deserialize)]
struct Slop {
    pattern: Vec<Pattern>,
}
#[derive(Deserialize)]
struct Pattern {
    id: String,
    kind: String,
    pattern: Option<String>,
    context: Option<String>,
    applies_to: Vec<String>,
    exempt: Option<String>,
}
pub struct Rule {
    pub id: String,
    pub re: Regex,
    pub replacement: Option<String>,
    pub context: bool,
    pub word_bound: bool,
    pub retired: bool,
    /// Ordinary uses of this variant that are not a name (`common_noun`, `key`).
    pub ordinary: Vec<String>,
    /// A term quoted, code-marked or negated in a sentence that forbids it is not a hit.
    pub forbidding: bool,
}
pub struct Catalogue {
    pub names: Names,
    pub rules: Vec<Rule>,
    slop: Vec<Pattern>,
    technical_forms: BTreeSet<String>,
}
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct Finding {
    pub file: PathBuf,
    pub line: usize,
    pub entry: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}
pub fn bounded(s: &str, insensitive: bool) -> Result<Regex> {
    Ok(Regex::new(&format!(
        "{}{}",
        if insensitive { "(?i)" } else { "" },
        regex::escape(s)
    ))?)
}
fn word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}
pub fn whole(text: &str, start: usize, end: usize) -> bool {
    !text[..start].chars().next_back().is_some_and(word)
        && !text[end..].chars().next().is_some_and(word)
}
/// Name matches cannot consume one component of a joined identifier or domain.
pub fn whole_name(text: &str, start: usize, end: usize) -> bool {
    if !whole(text, start, end) {
        return false;
    }
    let joined = |mut chars: Box<dyn Iterator<Item = char> + '_>| {
        chars
            .next()
            .is_some_and(|c| matches!(c, '.' | '-' | '_' | '/'))
            && chars.next().is_some_and(char::is_alphanumeric)
    };
    !joined(Box::new(text[..start].chars().rev())) && !joined(Box::new(text[end..].chars()))
}
fn identity_key(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}
fn sentence(text: &str, start: usize, end: usize) -> Result<(usize, usize)> {
    let block = Regex::new(r"^\s*(?:#{1,6}\s|[-*+]\s|[0-9]+[.)]\s|\|)")?;
    let mut stops = vec![0, text.len()];
    stops.extend(
        text.char_indices()
            .filter_map(|(i, c)| matches!(c, '.' | '!' | '?').then_some(i + 1)),
    );
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        if line.trim().is_empty() || block.is_match(line) {
            stops.push(offset);
        }
        if line.trim().is_empty() || line.trim_start().starts_with(['#', '|']) {
            stops.push(offset + line.len());
        }
        offset += line.len();
    }
    Ok((
        stops
            .iter()
            .copied()
            .filter(|p| *p <= start)
            .max()
            .unwrap_or(0),
        stops
            .into_iter()
            .filter(|p| *p >= end)
            .min()
            .unwrap_or(text.len()),
    ))
}
impl Catalogue {
    pub fn load(ns: &Namespace) -> Result<Self> {
        let (n, s) = ns.catalogues()?;
        let mut catalogue = Self::parse(&read(&n)?, &read(&s)?)?;
        catalogue.technical_forms.extend(
            ns.value["canonical_project_keys"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_owned),
        );
        Ok(catalogue)
    }
    pub fn parse(n: &str, s: &str) -> Result<Self> {
        let names: Names = toml::from_str(n)?;
        let slop: Slop = toml::from_str(s)?;
        let mut variants: Vec<(String, String, Option<String>, bool)> = vec![];
        for n in &names.name {
            for w in &n.wrong {
                variants.push((
                    w.clone(),
                    format!("name:{}", n.form),
                    Some(n.form.clone()),
                    false,
                ));
            }
        }
        for r in &names.retired {
            variants.push((
                r.name.clone(),
                format!("retired:{}", r.name),
                r.current.clone(),
                true,
            ));
        }
        variants.sort_by_key(|(s, _, _, _)| std::cmp::Reverse(s.len()));
        let is_retired = |s: &str| {
            names.retired.iter().any(|r| {
                r.name.eq_ignore_ascii_case(s)
                    && r.current
                        .as_ref()
                        .is_none_or(|c| identity_key(&r.name) != identity_key(c))
            })
        };
        let rules = variants
            .into_iter()
            .map(|(s, id, replacement, case)| {
                Ok(Rule {
                    id: id.clone(),
                    re: bounded(&s, case)?,
                    replacement: names
                        .retired
                        .iter()
                        .find(|r| r.name.eq_ignore_ascii_case(&s))
                        .map_or(replacement, |r| r.current.clone()),
                    retired: id.starts_with("retired:") || is_retired(&s),
                    ordinary: names
                        .name
                        .iter()
                        .filter(|n| id == format!("name:{}", n.form))
                        .flat_map(|n| n.ordinary.iter())
                        .filter(|o| o.wrong == s)
                        .map(|o| o.use_.clone())
                        .collect(),
                    forbidding: false,
                    context: false,
                    word_bound: true,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let technical_forms = names
            .name
            .iter()
            .flat_map(|n| [n.form.to_lowercase(), identity_key(&n.form)])
            .collect();
        Ok(Self {
            names,
            rules,
            technical_forms,
            slop: slop.pattern,
        })
    }
    pub fn prose_rules(&self, class: &str) -> Result<Vec<Rule>> {
        self.slop
            .iter()
            .filter(|p| p.applies_to.iter().any(|c| c == class) && p.kind != "judgment")
            .map(|p| {
                let pattern = p
                    .pattern
                    .as_deref()
                    .ok_or_else(|| crate::Error::Invalid(format!("{} has no pattern", p.id)))?;
                Ok(Rule {
                    id: p.id.clone(),
                    re: if p.kind == "phrase" {
                        bounded(pattern, true)?
                    } else {
                        Regex::new(pattern)?
                    },
                    replacement: None,
                    context: p.context.as_deref() == Some("name_change"),
                    word_bound: p.kind == "phrase",
                    retired: false,
                    ordinary: vec![],
                    forbidding: p.exempt.as_deref() == Some("forbidding"),
                })
            })
            .collect()
    }
    fn sentence_has_name(&self, s: &str) -> Result<bool> {
        for n in &self.names.name {
            if !n.form.chars().any(char::is_uppercase) && n.form != "herdr" {
                continue;
            }
            if bounded(&n.form, false)?
                .find_iter(s)
                .any(|m| whole_name(s, m.start(), m.end()))
            {
                return Ok(true);
            }
        }
        Ok(self
            .rules
            .iter()
            .any(|r| r.re.find_iter(s).any(|m| whole_name(s, m.start(), m.end()))))
    }
    pub fn findings(&self, path: &Path, text: &str, class: &str) -> Result<Vec<Finding>> {
        let components: Vec<_> = path.components().map(|c| c.as_os_str()).collect();
        let lineage = components
            .windows(2)
            .any(|p| p[0] == "00_constitution" && p[1] == "canon_chronicles");
        let prose = mask(text)?;
        let slop = self.prose_rules(class)?;
        let mut found = BTreeSet::new();
        for r in self
            .rules
            .iter()
            .filter(|r| {
                !lineage
                    && (!r.retired
                        || class == "core"
                            && !path.components().any(|c| {
                                c.as_os_str()
                                    .to_string_lossy()
                                    .eq_ignore_ascii_case("chronicles")
                                    || c.as_os_str()
                                        .to_string_lossy()
                                        .eq_ignore_ascii_case("canon_chronicles")
                            }))
            })
            .chain(slop.iter())
        {
            for m in r.re.find_iter(&prose) {
                let is_name = r.id.starts_with("name:") || r.id.starts_with("retired:");
                if (is_name && !whole_name(&prose, m.start(), m.end()))
                    || (!is_name && r.word_bound && !whole(&prose, m.start(), m.end()))
                {
                    continue;
                }
                if r.context {
                    let (start, end) = sentence(&prose, m.start(), m.end())?;
                    if !self.sentence_has_name(&prose[start..end])? {
                        continue;
                    }
                }
                if excused(r, path, text, m.start(), m.end())? {
                    continue;
                }
                found.insert((m.start(), m.end(), r.id.clone()));
            }
        }
        let code = code_regions(text)?;
        let chronicles = path.components().any(|c| {
            matches!(
                c.as_os_str().to_string_lossy().to_lowercase().as_str(),
                "chronicles" | "canon_chronicles"
            )
        });
        for r in self
            .rules
            .iter()
            .filter(|r| !lineage && (!r.retired || !chronicles))
            .chain(slop.iter())
        {
            for m in r.re.find_iter(text) {
                if !code
                    .iter()
                    .any(|(start, end)| m.start() >= *start && m.end() <= *end)
                    || !whole(text, m.start(), m.end())
                {
                    continue;
                }
                let token = m.as_str();
                let technical = !r.retired
                    && r.id.starts_with("name:")
                    && token.chars().any(char::is_lowercase)
                    && !token.chars().any(char::is_uppercase)
                    && self.technical_forms.contains(token);
                if technical {
                    continue;
                }
                if r.context {
                    let (start, end) = sentence(text, m.start(), m.end())?;
                    if !self.sentence_has_name(&text[start..end])? {
                        continue;
                    }
                }
                if excused(r, path, text, m.start(), m.end())? {
                    continue;
                }
                found.insert((m.start(), m.end(), r.id.clone()));
            }
        }
        Ok(found
            .into_iter()
            .map(|(start, _, entry)| Finding {
                file: path.into(),
                line: text[..start].bytes().filter(|b| *b == b'\n').count() + 1,
                entry,
                detail: None,
            })
            .collect())
    }
}
const DETERMINERS: &[&str] = &[
    "the", "a", "an", "its", "this", "that", "each", "every", "our", "his", "their", "whose",
];
const NEGATIONS: &[&str] = &[
    "no",
    "not",
    "never",
    "nothing",
    "without",
    "forbid",
    "forbids",
    "forbidden",
    "refuse",
    "refuses",
    "refused",
    "nor",
];
fn words_before(text: &str, start: usize) -> impl Iterator<Item = String> + '_ {
    text[..start]
        .split(|c: char| !word(c))
        .rev()
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
}
/// Narrow, catalogue-declared uses that are not the forbidden name or phrase.
fn excused(r: &Rule, path: &Path, text: &str, start: usize, end: usize) -> Result<bool> {
    let line_start = text[..start].rfind('\n').map_or(0, |n| n + 1);
    let line_end = text[end..].find('\n').map_or(text.len(), |n| end + n);
    let before = text[line_start..start].trim_end_matches([' ', '*', '_']);
    let after = &text[end..];
    if r.ordinary.iter().any(|u| u == "common_noun")
        && before.ends_with(|c: char| c.is_alphabetic())
        && words_before(text, start)
            .next()
            .is_some_and(|w| DETERMINERS.contains(&w.as_str()))
    {
        return Ok(true);
    }
    if r.ordinary.iter().any(|u| u == "key") {
        let mut next = after.chars();
        if next.next() == Some(':') && next.next().is_some_and(|c| !c.is_whitespace()) {
            return Ok(true);
        }
        // An all-lowercase parenthesised list names keys, not the product.
        let open = text[..start].rfind('(');
        let close_before = text[..start].rfind(')');
        if let Some(open) = open
            && close_before.is_none_or(|c| c < open)
            && start - open < 300
            && let Some(close) = after.find(')').filter(|c| *c < 300)
        {
            let inner = &text[open + 1..end + close];
            if !inner.chars().any(char::is_uppercase) && !inner.contains("\n\n") {
                return Ok(true);
            }
        }
    }
    if r.forbidding {
        let quoted = |o: char, c: char| before.ends_with(o) && after.starts_with(c);
        if quoted('"', '"')
            || quoted('“', '”')
            || quoted('\'', '\'')
            || quoted('‘', '’')
            || quoted('`', '`')
        {
            return Ok(true);
        }
        let (s, _) = sentence(text, start, end)?;
        if words_before(&text[s..], start - s)
            .take(6)
            .any(|w| NEGATIONS.contains(&w.as_str()))
        {
            return Ok(true);
        }
    }
    // A glossary entry that defines a retired name as retired is a historical mention.
    if r.retired
        && path
            .file_stem()
            .is_some_and(|s| s.to_string_lossy().contains("glossary"))
    {
        let line = text[line_start..line_end].trim_start();
        let line = line
            .strip_prefix(['-', '*', '+'])
            .unwrap_or(line)
            .trim_start();
        if let Some(term) = line.strip_prefix("**").and_then(|l| l.split("**").next())
            && term.eq_ignore_ascii_case(&text[start..end])
        {
            return Ok(true);
        }
    }
    Ok(false)
}
/// Code content is inspected separately from the prose/fix mask.
fn code_regions(text: &str) -> Result<Vec<(usize, usize)>> {
    let prefix = Regex::new(r"^(?:>\s*)*(?:(?:[-+*]|[0-9]+[.)])\s+)?")?;
    let mut regions = Vec::new();
    let mut fence = None;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let trim = line.trim_start();
        let n = prefix.find(trim).map_or(0, |m| m.end());
        let trim = trim[n..].trim_start();
        let c = trim.chars().next().unwrap_or(' ');
        let run = trim.chars().take_while(|x| *x == c).count();
        if let Some((fc, len)) = fence {
            if c == fc && run >= len && trim[run..].trim().is_empty() {
                fence = None;
            } else {
                regions.push((offset, offset + line.len()));
            }
        } else if matches!(c, '`' | '~') && run >= 3 {
            fence = Some((c, run));
        } else if line.starts_with("    ") || line.starts_with('\t') {
            regions.push((offset, offset + line.len()));
        }
        offset += line.len();
    }
    // Ignore fence delimiter lines while pairing inline spans, including language tags.
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if let Some((_, end)) = regions.iter().find(|(start, end)| i >= *start && i < *end) {
            i = *end;
            continue;
        }
        if bytes[i] != b'`' {
            i += 1;
            continue;
        }
        let line_start = text[..i].rfind('\n').map_or(0, |n| n + 1);
        let line_end = text[i..].find('\n').map_or(bytes.len(), |n| i + n);
        let line = &text[line_start..line_end];
        let trim = line.trim_start();
        let pre = prefix.find(trim).map_or(0, |m| m.end());
        let trim = trim[pre..].trim_start();
        if trim.starts_with("```") {
            i = line_end.saturating_add(1);
            continue;
        }
        let n = bytes[i..].iter().take_while(|x| **x == b'`').count();
        let mut j = i + n;
        let mut end = None;
        while j < bytes.len() {
            if bytes[j] == b'`' {
                let len = bytes[j..].iter().take_while(|x| **x == b'`').count();
                if len == n {
                    end = Some(j);
                    break;
                }
                j += len;
            } else {
                j += 1;
            }
        }
        if let Some(end) = end {
            regions.push((i + n, end));
            i = end + n;
        } else {
            i += n;
        }
    }
    Ok(regions)
}
pub fn is_document(p: &Path) -> bool {
    // CMakeLists.txt is a build script, not prose.
    matches!(p.extension().and_then(|x| x.to_str()), Some("md" | "txt"))
        && p.file_name().is_none_or(|n| n != "CMakeLists.txt")
}
/// Replace exempt bytes by spaces, retaining newlines and all byte offsets.
pub fn mask(text: &str) -> Result<String> {
    let mut b = text.as_bytes().to_vec();
    fn blank(b: &mut [u8], s: usize, e: usize) {
        for c in &mut b[s..e] {
            if *c != b'\n' && *c != b'\r' {
                *c = b' ';
            }
        }
    }
    let list_prefix = Regex::new(r"^(?:>\s*)*(?:(?:[-+*]|[0-9]+[.)])\s+)?")?;
    let mut offset = 0;
    let mut fence: Option<(char, usize)> = None;
    for line in text.split_inclusive('\n') {
        let trim = line.trim_start();
        let prefix = list_prefix.find(trim).map_or(0, |m| m.end());
        let trim = trim[prefix..].trim_start();
        let c = trim.chars().next().unwrap_or(' ');
        let run = trim.chars().take_while(|x| *x == c).count();
        if let Some((fc, n)) = fence {
            blank(&mut b, offset, offset + line.len());
            if c == fc && run >= n && trim[run..].trim().is_empty() {
                fence = None;
            }
        } else if (c == '`' || c == '~') && run >= 3 {
            fence = Some((c, run));
            blank(&mut b, offset, offset + line.len());
        } else if line.starts_with("    ") || line.starts_with('\t') {
            blank(&mut b, offset, offset + line.len());
        }
        offset += line.len();
    }
    // YAML front matter is document data rather than prose.
    if text.starts_with("---\n") || text.starts_with("---\r\n") {
        let mut end = text.find('\n').map_or(text.len(), |x| x + 1);
        for line in text[end..].split_inclusive('\n') {
            end += line.len();
            if matches!(line.trim(), "---" | "...") {
                blank(&mut b, 0, end);
                break;
            }
        }
    }
    // Source-located quotations retain the source's spelling.
    let lines: Vec<_> = text.split_inclusive('\n').collect();
    let mut pos = 0;
    let mut quoted = false;
    let literal_context = Regex::new(
        r"(?i)\b(?:files?|filenames?|references?|case[- ](?:in)?sensitive|regex|literals?|identifiers?|patterns?|search)\b",
    )?;
    let literals = Regex::new(r#"["“]([^"”\n]{1,100})["”]"#)?;
    for (i, line) in lines.iter().enumerate() {
        let trim = line.trim_start();
        if trim.starts_with('>') {
            if !quoted {
                quoted = lines[i.saturating_sub(3)..i].iter().any(|s| {
                    let lower = s.trim().to_lowercase();
                    lower.starts_with("from ") || lower.starts_with("source:")
                });
            }
            if quoted {
                blank(&mut b, pos, pos + line.len());
            }
        } else {
            quoted = false;
        }
        if literal_context.is_match(line) {
            for m in literals.find_iter(line) {
                blank(&mut b, pos + m.start(), pos + m.end());
            }
        }
        pos += line.len();
    }
    // Code-span delimiters must have equal lengths; unmatched ticks are prose.
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'`' {
            i += 1;
            continue;
        }
        let n = b[i..].iter().take_while(|x| **x == b'`').count();
        let mut j = i + n;
        let mut end = None;
        while j < b.len() {
            if b[j] == b'`' {
                let m = b[j..].iter().take_while(|x| **x == b'`').count();
                if m == n {
                    end = Some(j + m);
                    break;
                }
                j += m;
            } else {
                j += 1;
            }
        }
        if let Some(e) = end {
            blank(&mut b, i, e);
            i = e;
        } else {
            i += n;
        }
    }
    let interim = String::from_utf8(b.clone())
        .map_err(|_| crate::Error::Invalid("invalid prose mask".into()))?;
    // HTML comments are unrendered markers, not prose.
    for m in Regex::new(r"(?s)<!--.*?-->")?.find_iter(&interim) {
        blank(&mut b, m.start(), m.end());
    }
    // URLs, slash-containing path tokens and Markdown link destinations are data.
    let paths = Regex::new(
        r#"(?:https?://|file://|www\.)[^\s<>]+|\]\([^\n)]*\)|[^\s<>`\[\]()]*[/\\][^\s<>`\[\]()]*"#,
    )?;
    for m in paths.find_iter(&interim) {
        blank(&mut b, m.start(), m.end());
    }
    // A case-replacement map describes literal tokens rather than live names.
    let replacement_map = Regex::new(r"[\pL\pN_]+(?:→|->)[\pL\pN_]+")?;
    for m in replacement_map.find_iter(&interim) {
        blank(&mut b, m.start(), m.end());
    }
    let command_headings =
        Regex::new(r"(?m)^#{1,6}\s+[a-z][a-z0-9_-]*\s+[a-z][a-z0-9_-]*[^\n(]*\([^\n)]+\)\s*$")?;
    for m in command_headings.find_iter(&interim) {
        blank(&mut b, m.start(), m.end());
    }
    let commands = Regex::new(r"\b[a-z][a-z0-9_-]*\s+(?:serve|api)(?:\s+--[a-z0-9_-]+)?\b")?;
    for m in commands.find_iter(&interim) {
        blank(&mut b, m.start(), m.end());
    }
    let filenames = Regex::new(
        r"[\pL\pN_@.-]+\.(?:md|txt|rs|qml|json|toml|yaml|yml|sh|py|service|desktop|conf|tsv|sqlite|db)\b",
    )?;
    for m in filenames.find_iter(&interim) {
        blank(&mut b, m.start(), m.end());
    }
    String::from_utf8(b).map_err(|_| crate::Error::Invalid("invalid prose mask".into()))
}
/// Source-linked ledger examples remain records even when embedded in skills.
pub fn is_source_record(text: &str) -> bool {
    let first = text.lines().next().unwrap_or("").to_lowercase();
    let ledger = first.starts_with("# ledger: voice note")
        && text.lines().any(|s| s.starts_with("Source: "));
    let extraction = first.starts_with("# recovered from ")
        && first.contains("source chat:")
        && first.contains("nothing edited");
    (ledger || extraction) && text.contains("| Source lines |")
}

pub fn check(
    ns: &Namespace,
    paths: &[PathBuf],
    class: &str,
) -> Result<(Vec<Finding>, usize, Vec<PathBuf>)> {
    let report = inspect(ns, paths, class, None)?;
    Ok((report.findings, report.checked, report.refused_records))
}
#[derive(Debug, Serialize)]
pub struct CheckReport {
    pub findings: Vec<Finding>,
    pub checked: usize,
    pub refused_records: Vec<PathBuf>,
    pub allowed: Vec<crate::allowance::Allowed>,
    /// Files that are not UTF-8 text: counted and named, never read as prose.
    pub skipped: Vec<PathBuf>,
}
/// The timestamp filter uses filesystem modification time, never path spelling.
pub fn inspect(
    ns: &Namespace,
    paths: &[PathBuf],
    class: &str,
    changed_since: Option<&Path>,
) -> Result<CheckReport> {
    let cat = Catalogue::load(ns)?;
    let after = changed_since
        .map(|p| crate::io(p, std::fs::metadata(p).and_then(|m| m.modified())))
        .transpose()?;
    let mut report = CheckReport {
        findings: Vec::new(),
        checked: 0,
        refused_records: Vec::new(),
        allowed: Vec::new(),
        skipped: Vec::new(),
    };
    let mut core_files = Vec::new();
    let mut checked = std::collections::BTreeSet::new();
    for p in check_files(ns, paths, changed_since.is_some())? {
        if !is_document(&p) {
            continue;
        }
        if ns.value.pointer("/index/roles").is_some() && class_for(ns, &p) != class {
            continue;
        }
        if let Some(after) = after
            && (class_for(ns, &p) != class
                || crate::io(&p, std::fs::metadata(&p).and_then(|m| m.modified()))? <= after)
        {
            continue;
        }
        if class == "core" {
            core_files.push(p.clone());
        }
        if ns.is_record(&p) || is_skill_record(ns, &p) {
            report.refused_records.push(p);
            continue;
        }
        let Ok(text) = String::from_utf8(crate::io(&p, std::fs::read(&p))?) else {
            report.skipped.push(p);
            continue;
        };
        if is_source_record(&text) {
            report.refused_records.push(p);
            continue;
        }
        report.checked += 1;
        checked.insert(p.clone());
        report.findings.extend(cat.findings(&p, &text, class)?);
    }
    crate::allowance::apply(ns, &checked, &mut report, class)?;
    if class == "core" {
        report
            .findings
            .extend(crate::filenames::findings(ns, &core_files)?);
    }
    Ok(report)
}
/// Skill candidates and proof fixtures are pasted test records, listed but never corrected.
pub fn is_skill_record(ns: &Namespace, path: &Path) -> bool {
    ns.role("governance").is_ok_and(|g| {
        path.strip_prefix(g.join("skills")).is_ok_and(|rel| {
            rel.components()
                .nth(1)
                .is_some_and(|c| matches!(c.as_os_str().to_str(), Some("candidates" | "proof")))
        })
    })
}
/// Core membership follows the audit: canon, non-skill governance and front doors.
pub fn class_for(ns: &Namespace, path: &Path) -> &'static str {
    let rel = path.strip_prefix(&ns.root).unwrap_or(path);
    let file = rel.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let constitution = ns
        .role("constitution")
        .is_ok_and(|root| path.starts_with(root));
    let governance = ns
        .role("governance")
        .is_ok_and(|root| path.starts_with(&root) && !path.starts_with(root.join("skills")));
    if constitution || governance || matches!(file, "README.md" | "AGENTS.md" | "DIRECTORY_MAP.md")
    {
        "core"
    } else {
        "spine"
    }
}

/// Changed checks never descend into record containers or regenerable/heavy payloads.
fn check_files(ns: &Namespace, paths: &[PathBuf], changed: bool) -> Result<Vec<PathBuf>> {
    let policy = crate::index::Policy::load(ns)?;
    if !changed && policy.is_none() {
        return crate::files(paths);
    }
    fn visit(
        ns: &Namespace,
        policy: Option<&crate::index::Policy>,
        p: &Path,
        out: &mut Vec<PathBuf>,
    ) -> Result<()> {
        let metadata = crate::io(p, std::fs::symlink_metadata(p))?;
        if metadata.file_type().is_symlink() {
            return Ok(());
        }
        if policy.is_some_and(|policy| !policy.admits(p, metadata.is_dir())) {
            return Ok(());
        }
        if metadata.is_file() {
            if is_document(p) {
                out.push(crate::absolute(p)?);
            }
            return Ok(());
        }
        if ns.is_record(p) {
            return Ok(());
        }
        for entry in crate::io(p, std::fs::read_dir(p))? {
            let entry = crate::io(p, entry)?;
            visit(ns, policy, &entry.path(), out)?;
        }
        Ok(())
    }
    let mut out = Vec::new();
    for p in paths {
        visit(ns, policy.as_ref(), p, &mut out)?;
    }
    out.sort();
    out.dedup();
    Ok(out)
}
