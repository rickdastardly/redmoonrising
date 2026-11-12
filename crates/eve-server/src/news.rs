//! News article bodies: plain text with a small, safe set of HTML tags, so
//! articles can have headings, lists and highlights like the classic patch
//! notes. Anything else is shown as typed. The same body renders for the web
//! dashboard (styled by its stylesheet) and for the EVE client's character
//! selection browser (legacy HTML, colours as `<font>`).

/// Tags an article may use. `br` and `hr` stand alone; the rest are closed
/// for the writer if left open.
const TAGS: &[&str] = &[
    "h1", "h2", "h3", "h4", "b", "strong", "i", "em", "u", "ul", "ol", "li", "br", "hr", "p", "font", "a", "code", "small",
    "blockquote", "center", "big",
];
const VOID: &[&str] = &["br", "hr"];
/// Tags that start a new line themselves: a line break beside them is
/// layout, not a break the reader should see.
const BLOCK: &[&str] = &["h1", "h2", "h3", "h4", "ul", "ol", "li", "br", "hr", "p", "blockquote", "center"];

/// Title and heading colours for the client, where no stylesheet reaches.
pub const CLIENT_TITLE: &str = "#e8b04a";
const CLIENT_HEADING: &str = "#7cc4f4";
const CLIENT_COMMAND: &str = "#e8c060";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Dashboard,
    Client,
}

enum Piece {
    Text(String),
    Newline,
    Open { name: &'static str, attrs: String },
    Close(&'static str),
}

/// The body as safe HTML for `target`.
pub fn render(body: &str, target: Target) -> String {
    let pieces = parse(body);
    let mut out = String::new();
    let is_block = |p: &Piece| match p {
        Piece::Open { name, .. } | Piece::Close(name) => BLOCK.contains(name),
        _ => false,
    };
    let blank = |p: &Piece| matches!(p, Piece::Text(t) if t.trim().is_empty());
    for (i, piece) in pieces.iter().enumerate() {
        match piece {
            Piece::Text(t) => out.push_str(t),
            Piece::Newline => {
                let before = pieces[..i].iter().rev().find(|p| !blank(p));
                let after = pieces[i + 1..].iter().find(|p| !blank(p) && !matches!(p, Piece::Newline));
                let beside_block = before.is_some_and(is_block) || after.is_some_and(is_block);
                // A line break at the very start or end of the body shows nothing.
                if !beside_block && before.is_some() && after.is_some() {
                    out.push_str("<br>");
                } else {
                    out.push('\n');
                }
            }
            Piece::Open { name, attrs } => out.push_str(&open_tag(name, attrs, target)),
            Piece::Close(name) => out.push_str(&close_tag(name, target)),
        }
    }
    out
}

fn open_tag(name: &str, attrs: &str, target: Target) -> String {
    match (target, name) {
        (Target::Client, "h2") => format!("<h2><font color=\"{CLIENT_TITLE}\">"),
        (Target::Client, "h3") => format!("<h3><font color=\"{CLIENT_HEADING}\">"),
        (Target::Client, "code") => format!("<font color=\"{CLIENT_COMMAND}\">"),
        (Target::Client, "strong") => "<b>".into(),
        (Target::Client, "em") => "<i>".into(),
        (Target::Client, "big") => "<font size=\"+1\">".into(),
        _ => format!("<{name}{attrs}>"),
    }
}

fn close_tag(name: &str, target: Target) -> String {
    match (target, name) {
        (Target::Client, "h2") => "</font></h2>".into(),
        (Target::Client, "h3") => "</font></h3>".into(),
        (Target::Client, "code") => "</font>".into(),
        (Target::Client, "strong") => "</b>".into(),
        (Target::Client, "em") => "</i>".into(),
        (Target::Client, "big") => "</font>".into(),
        _ => format!("</{name}>"),
    }
}

fn parse(body: &str) -> Vec<Piece> {
    let body = body.replace("\r\n", "\n");
    let mut pieces = Vec::new();
    let mut open: Vec<&'static str> = Vec::new();
    let mut text = String::new();
    let mut rest = body.as_str();
    while let Some(c) = rest.chars().next() {
        if c == '<' {
            let tag = rest.find('>').map(|end| &rest[..=end]).filter(|t| !t[1..].contains('<'));
            if let Some(found) = tag.and_then(tag_piece) {
                if !text.is_empty() {
                    pieces.push(Piece::Text(std::mem::take(&mut text)));
                }
                match found {
                    Piece::Open { name, .. } => {
                        if !VOID.contains(&name) {
                            open.push(name);
                        }
                        pieces.push(found);
                    }
                    Piece::Close(name) => {
                        // A close for a tag that is not open is dropped; one for
                        // an outer tag closes the inner ones first.
                        if let Some(at) = open.iter().rposition(|n| *n == name) {
                            for inner in open.drain(at..).rev() {
                                pieces.push(Piece::Close(inner));
                            }
                        }
                    }
                    _ => {}
                }
                rest = &rest[tag.unwrap().len()..];
                continue;
            }
            text.push_str("&lt;");
        } else if c == '\n' {
            if !text.is_empty() {
                pieces.push(Piece::Text(std::mem::take(&mut text)));
            }
            pieces.push(Piece::Newline);
        } else if c == '&' {
            let entity = rest[1..].find(';').filter(|&n| n > 0 && n <= 8).map(|n| &rest[1..=n]).filter(|e| {
                e.chars().all(|c| c.is_ascii_alphanumeric()) || (e.starts_with('#') && e[1..].chars().all(|c| c.is_ascii_digit()) && e.len() > 1)
            });
            match entity {
                Some(e) => {
                    text.push('&');
                    text.push_str(e);
                    text.push(';');
                    rest = &rest[e.len() + 2..];
                    continue;
                }
                None => text.push_str("&amp;"),
            }
        } else if c == '>' {
            text.push_str("&gt;");
        } else if c == '"' {
            text.push_str("&quot;");
        } else {
            text.push(c);
        }
        rest = &rest[c.len_utf8()..];
    }
    if !text.is_empty() {
        pieces.push(Piece::Text(text));
    }
    for name in open.into_iter().rev() {
        pieces.push(Piece::Close(name));
    }
    pieces
}

/// `<name attrs>` or `</name>`, if it is an allowed tag with allowed attributes.
fn tag_piece(tag: &str) -> Option<Piece> {
    // "a < b > c" is text: a tag starts right after its bracket.
    if tag[1..].starts_with(char::is_whitespace) {
        return None;
    }
    let inner = tag[1..tag.len() - 1].trim();
    let (closing, inner) = match inner.strip_prefix('/') {
        Some(rest) => (true, rest.trim_start()),
        None => (false, inner),
    };
    let inner = inner.strip_suffix('/').unwrap_or(inner).trim_end();
    let name_len = inner.find(|c: char| !c.is_ascii_alphanumeric()).unwrap_or(inner.len());
    let lower = inner[..name_len].to_ascii_lowercase();
    let name = *TAGS.iter().find(|t| **t == lower)?;
    if closing {
        return (name_len == inner.len()).then_some(Piece::Close(name));
    }
    let attrs = attributes(name, &inner[name_len..])?;
    Some(Piece::Open { name, attrs })
}

/// The tag's attributes, rebuilt from the allowed ones; None if it has any other.
fn attributes(name: &str, mut rest: &str) -> Option<String> {
    let mut out = String::new();
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            return Some(out);
        }
        let key_len = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '-')).unwrap_or(rest.len());
        if key_len == 0 {
            return None;
        }
        let key = rest[..key_len].to_ascii_lowercase();
        rest = rest[key_len..].trim_start();
        rest = rest.strip_prefix('=')?.trim_start();
        let (value, after) = match rest.chars().next()? {
            q @ ('"' | '\'') => {
                let end = rest[1..].find(q)? + 1;
                (&rest[1..end], &rest[end + 1..])
            }
            _ => {
                let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                (&rest[..end], &rest[end..])
            }
        };
        rest = after;
        let ok = match (name, key.as_str()) {
            ("font", "color") => {
                let hex = value.strip_prefix('#').is_some_and(|h| matches!(h.len(), 3 | 6) && h.chars().all(|c| c.is_ascii_hexdigit()));
                hex || (!value.is_empty() && value.len() <= 20 && value.chars().all(|c| c.is_ascii_alphabetic()))
            }
            ("font", "size") => {
                let n = value.trim_start_matches(['+', '-']);
                n.len() == 1 && n.chars().all(|c| ('1'..='7').contains(&c))
            }
            ("a", "href") => {
                let v = value.to_ascii_lowercase();
                (v.starts_with("http://") || v.starts_with("https://")) && !value.contains(['"', '<', '>', ' '])
            }
            _ => false,
        };
        if !ok {
            return None;
        }
        out.push_str(&format!(" {key}=\"{value}\""));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_keeps_its_line_breaks_and_is_escaped() {
        assert_eq!(render("a < b & c\nnext\n\nafter", Target::Dashboard), "a &lt; b &amp; c<br>next<br><br>after");
    }

    #[test]
    fn layout_newlines_beside_blocks_add_no_gaps() {
        let body = "<h3>Logging in</h3>\n<ul>\n<li><b>One login</b> per account.</li>\n<li>Use <code>/ban</code>.</li>\n</ul>\nAfter.";
        assert_eq!(render(body, Target::Dashboard),
            "<h3>Logging in</h3>\n<ul>\n<li><b>One login</b> per account.</li>\n<li>Use <code>/ban</code>.</li>\n</ul>\nAfter.");
        let client = render(body, Target::Client);
        assert!(client.starts_with(&format!("<h3><font color=\"{CLIENT_HEADING}\">Logging in</font></h3>")));
        assert!(client.contains(&format!("<font color=\"{CLIENT_COMMAND}\">/ban</font>")));
    }

    #[test]
    fn unknown_tags_and_attributes_show_as_text() {
        assert_eq!(render("<script>x</script>", Target::Dashboard), "&lt;script&gt;x&lt;/script&gt;");
        assert_eq!(render("a < b > c", Target::Dashboard), "a &lt; b &gt; c");
        assert_eq!(render("<b onclick=\"x\">y</b>", Target::Dashboard), "&lt;b onclick=&quot;x&quot;&gt;y");
        assert_eq!(render("<a href=\"javascript:x\">y</a>", Target::Dashboard), "&lt;a href=&quot;javascript:x&quot;&gt;y");
        assert_eq!(render("<a href=\"https://example.com/\">y</a>", Target::Dashboard), "<a href=\"https://example.com/\">y</a>");
        assert_eq!(render("<font color='#ff0' size=+1>y</font>", Target::Dashboard), "<font color=\"#ff0\" size=\"+1\">y</font>");
    }

    #[test]
    fn unclosed_tags_are_closed_and_stray_closes_dropped() {
        assert_eq!(render("<b>bold <i>both", Target::Dashboard), "<b>bold <i>both</i></b>");
        assert_eq!(render("x</b><ul><li>a</ul>", Target::Dashboard), "x<ul><li>a</li></ul>");
        assert_eq!(render("&amp; &#39; &nbsp; &bogus", Target::Dashboard), "&amp; &#39; &nbsp; &amp;bogus");
    }
}
