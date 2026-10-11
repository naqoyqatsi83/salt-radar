//! Event sniffer terms, parsed from the comma-separated input that starts a
//! sniff (or narrows one with `h`): minion IDs and globs, plain keywords and
//! `/regex/` terms.

use regex::{Regex, RegexBuilder};
use serde_json::Value;

use crate::types::SaltEvent;

#[derive(Clone, Debug)]
pub enum SniffTerm {
    /// A known minion ID, or a glob matching known minions (`ids`, resolved
    /// when the term was parsed).
    Minion { pattern: String, ids: Vec<String> },
    /// Matches its text anywhere, ignoring case. `lower` is kept alongside
    /// the text as typed, which colors it.
    Keyword { text: String, lower: String },
    Regex(Regex),
}

impl SniffTerm {
    /// The term as typed, which its highlight color is derived from — one
    /// color per term, a glob's minions included.
    pub fn color_key(&self) -> &str {
        match self {
            SniffTerm::Minion { pattern, .. } => pattern,
            SniffTerm::Keyword { text, .. } => text,
            SniffTerm::Regex(re) => re.as_str(),
        }
    }

    /// Byte ranges of this term's matches in `text`.
    pub fn find_in(&self, text: &str) -> Vec<(usize, usize)> {
        match self {
            SniffTerm::Minion { ids, .. } => ids.iter().flat_map(|id| find_ignore_case(text, &id.to_ascii_lowercase())).collect(),
            SniffTerm::Keyword { lower, .. } => find_ignore_case(text, lower),
            SniffTerm::Regex(re) => re.find_iter(text).filter(|m| !m.is_empty()).map(|m| (m.start(), m.end())).collect(),
        }
    }
}

/// `needle` must already be lowercase; ASCII lowercasing keeps byte offsets.
fn find_ignore_case(text: &str, needle: &str) -> Vec<(usize, usize)> {
    if needle.is_empty() {
        return Vec::new();
    }
    let lower = text.to_ascii_lowercase();
    lower.match_indices(needle).map(|(i, m)| (i, i + m.len())).collect()
}

/// Whether what follows a `/` makes it a regex's closing slash: an optional
/// `i`, then only whitespace up to a comma or the end.
fn ends_regex(rest: &[char]) -> bool {
    let rest = rest.strip_prefix(&['i']).unwrap_or(rest);
    rest.iter().find(|c| !c.is_whitespace()).is_none_or(|&c| c == ',')
}

enum RawTerm {
    Text(String),
    Regex { pattern: String, ignore_case: bool },
}

/// Splits the input on commas, except inside a `/…/` term. A regex ends at
/// the `/` that is followed by an optional `i` flag and then a comma or the
/// end of the input, so slashes inside it (event tags are full of them)
/// need no escaping; `\/` is a literal slash too.
fn split_terms(input: &str) -> Result<Vec<RawTerm>, String> {
    let chars: Vec<char> = input.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() || chars[i] == ',' {
            i += 1;
            continue;
        }
        if chars[i] == '/' {
            i += 1;
            let mut pattern = String::new();
            let mut closed = false;
            while i < chars.len() {
                match chars[i] {
                    '\\' if chars.get(i + 1) == Some(&'/') => {
                        pattern.push('/');
                        i += 2;
                    }
                    '\\' if i + 1 < chars.len() => {
                        pattern.push('\\');
                        pattern.push(chars[i + 1]);
                        i += 2;
                    }
                    '/' if ends_regex(&chars[i + 1..]) => {
                        closed = true;
                        i += 1;
                        break;
                    }
                    c => {
                        pattern.push(c);
                        i += 1;
                    }
                }
            }
            if !closed {
                return Err(format!("/{pattern}: missing the closing / (or /i) before a comma or the end"));
            }
            let mut flags = String::new();
            while i < chars.len() && chars[i] != ',' {
                flags.push(chars[i]);
                i += 1;
            }
            let ignore_case = flags.trim() == "i";
            if pattern.is_empty() {
                return Err("//: empty regex".to_string());
            }
            out.push(RawTerm::Regex { pattern, ignore_case });
        } else {
            let mut text = String::new();
            while i < chars.len() && chars[i] != ',' {
                text.push(chars[i]);
                i += 1;
            }
            let text = text.trim();
            if !text.is_empty() {
                out.push(RawTerm::Text(text.to_string()));
            }
        }
    }
    Ok(out)
}

/// Parses sniffer input. `minions` resolves a plain term to the known
/// minions it names (an exact ID, or a glob); a term it resolves to nothing
/// is a keyword.
pub fn parse_terms(input: &str, minions: impl Fn(&str) -> Vec<String>) -> Result<Vec<SniffTerm>, String> {
    split_terms(input)?.into_iter().map(|raw| match raw {
        RawTerm::Text(text) => {
            let ids = minions(&text);
            Ok(if ids.is_empty() {
                SniffTerm::Keyword { lower: text.to_ascii_lowercase(), text }
            } else {
                SniffTerm::Minion { pattern: text, ids }
            })
        }
        RawTerm::Regex { pattern, ignore_case } => RegexBuilder::new(&pattern)
            .case_insensitive(ignore_case)
            .build()
            .map(SniffTerm::Regex)
            .map_err(|e| format!("/{pattern}/: {}", e.to_string().lines().last().unwrap_or("invalid regex").trim().trim_start_matches("error: "))),
    }).collect()
}

/// Whether `evt` matches any of `terms`. Keywords and regexes are matched
/// against the event as the sniffer shows it — its tag and its
/// pretty-printed JSON, line by line — so whatever matches is also colored.
pub fn matches(terms: &[SniffTerm], evt: &SaltEvent) -> bool {
    let mut shown: Option<Vec<String>> = None;
    terms.iter().any(|term| match term {
        SniffTerm::Minion { ids, .. } => concerns_minion(evt, ids),
        SniffTerm::Keyword { lower, .. } => shown_lines(&mut shown, evt).iter().any(|l| l.to_ascii_lowercase().contains(lower.as_str())),
        SniffTerm::Regex(re) => shown_lines(&mut shown, evt).iter().any(|l| re.is_match(l)),
    })
}

fn shown_lines<'a>(cache: &'a mut Option<Vec<String>>, evt: &SaltEvent) -> &'a [String] {
    cache.get_or_insert_with(|| {
        let json = serde_json::to_string_pretty(&evt.data).unwrap_or_default();
        std::iter::once(evt.tag.clone()).chain(json.lines().map(String::from)).collect()
    })
}

/// An event from or about one of `ids`: sent by it, naming it in the tag, a
/// job targeting it (by the master's resolved `minions` list, or a `*` / glob
/// / list target), or mentioning it in any value.
fn concerns_minion(evt: &SaltEvent, ids: &[String]) -> bool {
    let is_one = |s: &str| ids.iter().any(|id| id == s);
    let data = &evt.data;
    if data["id"].as_str().is_some_and(is_one) || evt.tag.split('/').any(is_one) {
        return true;
    }
    if data["minions"].as_array().is_some_and(|m| m.iter().filter_map(Value::as_str).any(is_one)) {
        return true;
    }
    let glob_target = matches!(data["tgt_type"].as_str(), None | Some("glob"));
    match &data["tgt"] {
        Value::String(t) if glob_target && ids.iter().any(|id| crate::app::App::glob_match(id, t)) => return true,
        Value::Array(list) if list.iter().filter_map(Value::as_str).any(is_one) => return true,
        _ => {}
    }
    mentions(data, ids)
}

fn mentions(value: &Value, ids: &[String]) -> bool {
    match value {
        Value::String(s) => ids.iter().any(|id| s.contains(id.as_str())),
        Value::Object(map) => map.values().any(|v| mentions(v, ids)),
        Value::Array(arr) => arr.iter().any(|v| mentions(v, ids)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn evt(tag: &str, data: Value) -> SaltEvent {
        SaltEvent { tag: tag.into(), data, ts: 0.0 }
    }

    fn known(t: &str) -> Vec<String> {
        let all = ["web1", "web2", "db1"];
        all.iter().filter(|m| crate::app::App::glob_match(m, t)).map(|m| m.to_string()).collect()
    }

    fn kinds(input: &str) -> Vec<String> {
        parse_terms(input, known).unwrap().iter().map(|t| match t {
            SniffTerm::Minion { pattern, ids } => format!("minion {pattern}={}", ids.join("+")),
            SniffTerm::Keyword { text, .. } => format!("keyword {text}"),
            SniffTerm::Regex(re) => format!("regex {}", re.as_str()),
        }).collect()
    }

    #[test]
    fn terms_split_on_commas_but_not_inside_a_regex() {
        assert_eq!(kinds(" web*, nginx ,, /a{1,3}/i, db1,/x\\/y/"), [
            "minion web*=web1+web2", "keyword nginx", "regex a{1,3}", "minion db1=db1", "regex x/y",
        ]);
        assert_eq!(kinds("web9*"), ["keyword web9*"], "a glob matching no minion is a keyword");
        assert_eq!(kinds(r"/salt/job/\d+/ret/(db|web)\d/i , nginx"), [r"regex salt/job/\d+/ret/(db|web)\d", "keyword nginx"]);
    }

    #[test]
    fn bad_regex_terms_are_errors() {
        assert!(parse_terms("/abc", known).unwrap_err().contains("missing the closing /"));
        assert!(parse_terms("/abc/x", known).unwrap_err().contains("missing the closing /"), "x is not a flag");
        assert!(parse_terms("//", known).unwrap_err().contains("empty"));
        assert!(parse_terms("/[a-/", known).unwrap_err().starts_with("/[a-/:"));
    }

    #[test]
    fn keywords_ignore_case_and_skip_star_jobs() {
        let terms = parse_terms("NGINX", known).unwrap();
        assert!(matches(&terms, &evt("salt/job/1/ret/web1", json!({"return": "nginx running"}))));
        assert!(!matches(&terms, &evt("salt/job/1/new", json!({"tgt": "*", "fun": "test.ping"}))));
    }

    #[test]
    fn minion_terms_follow_their_jobs() {
        let terms = parse_terms("web*", known).unwrap();
        assert!(matches(&terms, &evt("salt/job/1/new", json!({"tgt": "*", "fun": "test.ping"}))));
        assert!(matches(&terms, &evt("salt/job/2/new", json!({"tgt": "G@os:Debian", "tgt_type": "compound", "minions": ["web2"]}))));
        assert!(matches(&terms, &evt("salt/job/3/ret/web1", json!({"id": "web1"}))));
        assert!(!matches(&terms, &evt("salt/job/4/ret/db1", json!({"id": "db1"}))));
        assert!(!matches(&terms, &evt("salt/job/5/new", json!({"tgt": "db*", "minions": ["db1"]}))));
    }

    #[test]
    fn regex_matches_the_shown_json_line_by_line() {
        let terms = parse_terms(r#"/"retcode": [1-9]/"#, known).unwrap();
        assert!(matches(&terms, &evt("salt/job/1/ret/web1", json!({"retcode": 2}))));
        assert!(!matches(&terms, &evt("salt/job/1/ret/web1", json!({"retcode": 0}))));
        assert_eq!(terms[0].find_in(r#"  "retcode": 2,"#), [(2, 14)]);
    }

    #[test]
    fn find_in_finds_every_minion_of_a_glob() {
        let terms = parse_terms("web*", known).unwrap();
        assert_eq!(terms[0].find_in("web1 and WEB2"), [(0, 4), (9, 13)]);
        assert_eq!(terms[0].color_key(), "web*");
    }
}
