//! Markdown + YAML-frontmatter parsing. Pure functions over a note's text.
//!
//! Files stay the source of truth, so parsing is read-only and never rewrites
//! the user's Markdown.

use std::sync::LazyLock;

use pulldown_cmark::{CowStr, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use regex::Regex;
use serde_json::Value;

use crate::model::{Heading, Link, LinkKind, ParsedNote};

static WIKILINK: LazyLock<Regex> =
    // optional leading `!` (embed) + [[ inner ]]
    LazyLock::new(|| Regex::new(r"(!?)\[\[([^\[\]\n]+)\]\]").unwrap());

static TAG: LazyLock<Regex> =
    // `#tag`, must follow start/space/paren, start with a letter, allow nesting a/b
    LazyLock::new(|| Regex::new(r"(?:^|[\s(])#(\p{L}[\p{L}\p{N}_/-]*)").unwrap());

static FENCED_CODE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)```.*?```|~~~.*?~~~").unwrap());

static INLINE_CODE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"`[^`\n]*`").unwrap());

/// Render a Markdown body to HTML (GFM). Used by `satchel export --html`.
pub fn to_html(markdown: &str) -> String {
    use pulldown_cmark::{html, Options, Parser};
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_FOOTNOTES);
    let parser = Parser::new_ext(markdown, opts);
    let mut out = String::new();
    html::push_html(&mut out, parser);
    out
}

/// Parse a note's full text (frontmatter + body) into its derived parts.
pub fn parse(content: &str) -> ParsedNote {
    let (frontmatter, body) = split_frontmatter(content);
    let (headings, plaintext) = walk_body(&body);

    let scannable = strip_code(&body);
    let links = extract_links(&scannable);
    let mut tags = frontmatter_tags(&frontmatter);
    for cap in TAG.captures_iter(&scannable) {
        push_unique(&mut tags, cap[1].to_string());
    }

    let title = title_from(&frontmatter, &headings);

    ParsedNote {
        title,
        frontmatter,
        body,
        headings,
        links,
        tags,
        plaintext,
    }
}

/// Split a leading `---` … `---` YAML frontmatter block from the body.
/// Returns `(Value::Null, whole)` when there is no valid frontmatter.
fn split_frontmatter(content: &str) -> (Value, String) {
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    let after_open = match content
        .strip_prefix("---\r\n")
        .or_else(|| content.strip_prefix("---\n"))
    {
        Some(rest) => rest,
        None => return (Value::Null, content.to_string()),
    };

    let mut offset = 0usize;
    for line in after_open.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed == "---" || trimmed == "..." {
            let yaml = &after_open[..offset];
            let body = &after_open[offset + line.len()..];
            let fm = serde_yaml_ng::from_str::<Value>(yaml).unwrap_or(Value::Null);
            let fm = if fm.is_object() { fm } else { Value::Null };
            return (fm, body.to_string());
        }
        offset += line.len();
    }
    // Unterminated frontmatter → treat as plain body.
    (Value::Null, content.to_string())
}

fn h_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// Walk the markdown AST to collect headings and a markup-stripped plaintext
/// projection (for full-text search and snippets).
fn walk_body(body: &str) -> (Vec<Heading>, String) {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_FOOTNOTES);

    let mut headings = Vec::new();
    let mut plaintext = String::new();
    let mut in_heading: Option<u8> = None;
    let mut heading_text = String::new();

    let push_text = |buf: &mut String, t: &CowStr| {
        buf.push_str(t);
        buf.push(' ');
    };

    for event in Parser::new_ext(body, opts) {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                in_heading = Some(h_level(level));
                heading_text.clear();
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some(level) = in_heading.take() {
                    let text = heading_text.trim().to_string();
                    plaintext.push_str(&text);
                    plaintext.push(' ');
                    headings.push(Heading { level, text });
                }
            }
            Event::Text(t) | Event::Code(t) => {
                if in_heading.is_some() {
                    heading_text.push_str(&t);
                } else {
                    push_text(&mut plaintext, &t);
                }
            }
            _ => {}
        }
    }

    (headings, normalize_ws(&plaintext))
}

/// Remove fenced and inline code so wikilinks/tags inside code aren't picked up.
fn strip_code(body: &str) -> String {
    let no_fenced = FENCED_CODE.replace_all(body, " ");
    INLINE_CODE.replace_all(&no_fenced, " ").into_owned()
}

fn extract_links(scannable: &str) -> Vec<Link> {
    let mut links: Vec<Link> = Vec::new();
    for cap in WIKILINK.captures_iter(scannable) {
        let kind = if &cap[1] == "!" {
            LinkKind::Embed
        } else {
            LinkKind::Wikilink
        };
        let inner = &cap[2];
        let (left, alias) = match inner.split_once('|') {
            Some((l, a)) => (l, Some(a.trim().to_string())),
            None => (inner, None),
        };
        let (target, heading) = match left.split_once('#') {
            Some((t, h)) => (t.trim().to_string(), Some(h.trim().to_string())),
            None => (left.trim().to_string(), None),
        };
        if target.is_empty() && heading.is_none() {
            continue;
        }
        let link = Link {
            target,
            heading,
            alias,
            kind,
        };
        if !links.contains(&link) {
            links.push(link);
        }
    }
    links
}

/// Collect tags declared in frontmatter (`tags:` as a list or a delimited string).
fn frontmatter_tags(fm: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let Some(tags) = fm.get("tags") else {
        return out;
    };
    match tags {
        Value::Array(items) => {
            for item in items {
                // Coerce scalar tags (strings, numbers like a year, bools).
                let s = match item {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    Value::Bool(b) => b.to_string(),
                    _ => continue,
                };
                push_unique(&mut out, s.trim_start_matches('#').trim().to_string());
            }
        }
        Value::String(s) => {
            for part in s.split([',', ' ']) {
                let t = part.trim_start_matches('#').trim();
                if !t.is_empty() {
                    push_unique(&mut out, t.to_string());
                }
            }
        }
        _ => {}
    }
    out
}

fn title_from(fm: &Value, headings: &[Heading]) -> String {
    if let Some(t) = fm.get("title").and_then(Value::as_str) {
        let t = t.trim();
        if !t.is_empty() {
            return t.to_string();
        }
    }
    headings
        .iter()
        .find(|h| h.level == 1)
        .map(|h| h.text.clone())
        .unwrap_or_default()
}

fn push_unique(v: &mut Vec<String>, s: String) {
    if !s.is_empty() && !v.contains(&s) {
        v.push(s);
    }
}

fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frontmatter_title_and_tags() {
        let md = "---\ntitle: My Note\ntags: [project, rust]\n---\n# Heading\n\nbody text\n";
        let p = parse(md);
        assert_eq!(p.title, "My Note");
        assert!(p.tags.contains(&"project".to_string()));
        assert!(p.tags.contains(&"rust".to_string()));
        assert_eq!(p.frontmatter.get("title").unwrap(), "My Note");
        assert!(!p.body.contains("title: My Note"), "frontmatter stripped from body");
    }

    #[test]
    fn title_falls_back_to_first_h1() {
        let p = parse("# Real Title\n\ntext");
        assert_eq!(p.title, "Real Title");
    }

    #[test]
    fn extracts_three_wikilink_forms() {
        let md = "See [[Plain]], [[Target#Section]], and [[Note|an alias]]. Also ![[image.png]].";
        let p = parse(md);
        let plain = p.links.iter().find(|l| l.target == "Plain").unwrap();
        assert_eq!(plain.kind, LinkKind::Wikilink);
        assert!(plain.heading.is_none() && plain.alias.is_none());

        let with_heading = p.links.iter().find(|l| l.target == "Target").unwrap();
        assert_eq!(with_heading.heading.as_deref(), Some("Section"));

        let with_alias = p.links.iter().find(|l| l.target == "Note").unwrap();
        assert_eq!(with_alias.alias.as_deref(), Some("an alias"));

        let embed = p.links.iter().find(|l| l.target == "image.png").unwrap();
        assert_eq!(embed.kind, LinkKind::Embed);
    }

    #[test]
    fn finds_inline_tags_and_merges_with_frontmatter() {
        let md = "---\ntags:\n  - alpha\n---\nbody with #beta and #nested/tag here";
        let p = parse(md);
        assert!(p.tags.contains(&"alpha".to_string()));
        assert!(p.tags.contains(&"beta".to_string()));
        assert!(p.tags.contains(&"nested/tag".to_string()));
    }

    #[test]
    fn ignores_wikilinks_and_tags_inside_code() {
        let md = "real [[Live]] #realtag\n\n```\nnot [[Fake]] #faketag\n```\n\ninline `[[AlsoFake]]` too";
        let p = parse(md);
        assert!(p.links.iter().any(|l| l.target == "Live"));
        assert!(!p.links.iter().any(|l| l.target == "Fake"));
        assert!(!p.links.iter().any(|l| l.target == "AlsoFake"));
        assert!(p.tags.contains(&"realtag".to_string()));
        assert!(!p.tags.contains(&"faketag".to_string()));
    }

    #[test]
    fn plaintext_strips_markup() {
        let p = parse("# Title\n\nSome **bold** and a [link](http://x).");
        // No markdown punctuation like ** or [] in the plaintext projection.
        assert!(!p.plaintext.contains('*'));
        assert!(!p.plaintext.contains('['));
        assert!(p.plaintext.contains("bold"));
        assert!(p.plaintext.contains("Title"));
    }

    #[test]
    fn no_frontmatter_is_fine() {
        let p = parse("just text, no fm");
        assert!(p.frontmatter.is_null());
        assert_eq!(p.body, "just text, no fm");
    }
}
