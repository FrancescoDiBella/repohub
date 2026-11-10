//! README rendering: GitHub-flavoured Markdown prepared for `rune_kit::Markdown`.
//!
//! GitHub READMEs lean on things a plain Markdown renderer can't show as written:
//! - relative links (`docs/setup.md`) become absolute links to the file on github.com;
//! - images become links to the picture (badges and screenshots aren't downloaded);
//! - raw HTML (`<p align="center">`, `<img>`, comments) is dropped, keeping any text inside.

/// Turns `src` (a README from `repo` at `branch`) into Markdown the renderer can show.
pub fn prepare(src: &str, repo: &str, branch: &str) -> String {
    let blob = format!("https://github.com/{repo}/blob/{branch}/");
    let raw = format!("https://github.com/{repo}/raw/{branch}/");
    let mut out = String::with_capacity(src.len());
    let mut fence: Option<&str> = None;
    let mut in_comment = false;
    for line in src.lines() {
        let t = line.trim_start();
        // Code fences pass through untouched.
        if let Some(f) = fence {
            out.push_str(line);
            out.push('\n');
            if t.starts_with(f) {
                fence = None;
            }
            continue;
        }
        if t.starts_with("```") || t.starts_with("~~~") {
            fence = Some(if t.starts_with("```") { "```" } else { "~~~" });
            out.push_str(line);
            out.push('\n');
            continue;
        }
        let mut l = line.to_owned();
        if in_comment {
            match l.find("-->") {
                Some(i) => {
                    l = l[i + 3..].to_owned();
                    in_comment = false;
                }
                None => continue,
            }
        }
        while let Some(i) = l.find("<!--") {
            match l[i..].find("-->") {
                Some(j) => l.replace_range(i..i + j + 3, ""),
                None => {
                    l.truncate(i);
                    in_comment = true;
                }
            }
        }
        let l = if let Some(def) = reference(&l, &blob) {
            def
        } else {
            outside_code(&l, |part| rewrite_links(&strip_tags(part), &blob, &raw))
        };
        // Lines that were only HTML collapse to nothing; keep paragraph breaks single.
        if l.trim().is_empty() && line.trim().starts_with('<') {
            continue;
        }
        out.push_str(&l);
        out.push('\n');
    }
    out
}

/// Splits a line into text outside and inside inline code spans (`` `like this` ``), so code
/// such as `Vec<String>` or `a[i](x)` is left exactly as written.
fn outside_code(l: &str, f: impl Fn(&str) -> String) -> String {
    let mut out = String::new();
    let mut rest = l;
    while let Some(i) = rest.find('`') {
        let ticks = rest[i..].bytes().take_while(|b| *b == b'`').count();
        let fence = &rest[i..i + ticks];
        let Some(j) = rest[i + ticks..].find(fence) else { break };
        let end = i + ticks + j + ticks;
        out.push_str(&f(&rest[..i]));
        out.push_str(&rest[i..end]);
        rest = &rest[end..];
    }
    out.push_str(&f(rest));
    out
}

/// The value of attribute `name` in an HTML tag, with either kind of quotes.
fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    for q in ['"', '\''] {
        let k = format!("{name}={q}");
        // The attribute name must start a word (`src=`, not `data-src=`).
        let found = lower.match_indices(&k).find(|(a, _)| *a == 0 || lower.as_bytes()[a - 1].is_ascii_whitespace());
        if let Some((a, _)) = found {
            return tag[a + k.len()..].split(q).next().map(str::to_owned);
        }
    }
    None
}

/// Removes HTML tags, keeping the text between them. `<img>` and `<a href>` become Markdown
/// images and links, `<h1>`–`<h6>` headings and `<br>` a space; autolinks (`<https://…>`) stay.
fn strip_tags(l: &str) -> String {
    let mut out = String::new();
    let mut rest = l;
    // The target of the `<a href>` that is open, closed by `</a>` on the same line.
    let mut link: Option<String> = None;
    while let Some(i) = rest.find('<') {
        let tail = &rest[i + 1..];
        let is_tag = tail.starts_with(|c: char| c.is_ascii_alphabetic() || c == '/' || c == '!');
        let Some(j) = tail.find('>').filter(|_| is_tag) else {
            out.push_str(&rest[..=i]);
            rest = &rest[i + 1..];
            continue;
        };
        let tag = &tail[..j];
        // `<https://example.com>` and `<me@example.com>` are Markdown autolinks, not HTML.
        if !tag.contains(char::is_whitespace) && (tag.contains("://") || tag.contains('@') || tag.starts_with("mailto:")) {
            out.push_str(&rest[..i + 1 + j + 1]);
            rest = &tail[j + 1..];
            continue;
        }
        out.push_str(&rest[..i]);
        let lower = tag.to_ascii_lowercase();
        let name: String = lower.trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
        let closing = lower.starts_with('/');
        match name.as_str() {
            "img" => {
                if let Some(src) = attr(tag, "src") {
                    let alt = attr(tag, "alt").filter(|a| !a.is_empty()).unwrap_or_else(|| "image".into());
                    // Inside a link (a badge), the link's own target wins: keep only the label.
                    if link.is_some() { out.push_str(&alt) } else { out.push_str(&format!("![{alt}]({src})")) }
                }
            }
            "a" if !closing => {
                if let Some(href) = attr(tag, "href").filter(|_| tail[j + 1..].to_ascii_lowercase().contains("</a")) {
                    out.push('[');
                    link = Some(href);
                }
            }
            "a" => {
                if let Some(href) = link.take() {
                    out.push_str(&format!("]({href})"));
                }
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" if !closing && out.trim().is_empty() => {
                let level = (name.as_bytes()[1] - b'0') as usize;
                out.push_str(&"#".repeat(level));
                out.push(' ');
            }
            "br" => out.push(' '),
            _ => {}
        }
        rest = &tail[j + 1..];
    }
    out.push_str(rest);
    out
}

/// A link reference definition (`[docs]: docs/setup.md`) with its target made absolute.
fn reference(l: &str, blob: &str) -> Option<String> {
    let t = l.trim_start();
    if l.len() - t.len() > 3 || !t.starts_with('[') || t.starts_with("[^") {
        return None;
    }
    let (label, rest) = t.split_once("]:")?;
    if label.contains(']') {
        return None;
    }
    let rest = rest.trim();
    let (url, title) = rest.split_once(' ').map(|(u, t)| (u, format!(" {t}"))).unwrap_or((rest, String::new()));
    let url = url.trim_start_matches('<').trim_end_matches('>');
    (!url.is_empty()).then(|| format!("{label}]: {}{title}", absolute(url, blob)))
}

/// `url` as it is when it already points somewhere, else under `base`.
fn absolute(url: &str, base: &str) -> String {
    if url.contains("://") || url.starts_with("mailto:") || url.starts_with('#') || url.starts_with("//") {
        url.to_owned()
    } else {
        format!("{base}{}", url.trim_start_matches("./").trim_start_matches('/'))
    }
}

/// Makes Markdown link and image targets absolute; images become links to the picture.
fn rewrite_links(l: &str, blob: &str, raw: &str) -> String {
    let mut out = String::new();
    let mut rest = l;
    while let Some(i) = rest.find("](") {
        let Some(close) = rest[i + 2..].find(')') else { break };
        let label_start = rest[..i].rfind('[');
        let target = rest[i + 2..i + 2 + close].trim();
        let (url, title) = match target.split_once(' ') {
            Some((u, t)) => (u, format!(" {t}")),
            None => (target, String::new()),
        };
        let is_image = label_start.is_some_and(|s| s > 0 && rest.as_bytes()[s - 1] == b'!');
        let absolute = |base: &str| absolute(url, base);
        match (is_image, label_start) {
            (true, Some(s)) => {
                let alt = &rest[s + 1..i];
                out.push_str(&rest[..s - 1]);
                let alt = if alt.is_empty() { "image" } else { alt };
                out.push_str(&format!("[Image: {alt}]({}{title})", absolute(raw)));
            }
            _ => {
                out.push_str(&rest[..i + 2]);
                out.push_str(&absolute(blob));
                out.push_str(&title);
                out.push(')');
            }
        }
        rest = &rest[i + 2 + close + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::prepare;

    #[test]
    fn github_readme_bits() {
        let src = "<p align=\"center\">\n  <img src=\"docs/logo.png\" alt=\"Logo\" width=\"80\">\n</p>\n\n# Title <!-- hidden -->\n\nSee [setup](docs/setup.md) and [site](https://x.dev).\n\n![Screen](docs/shot.png)\n\n```html\n<div>[keep](as-is)</div>\n```\n";
        let out = prepare(src, "octo/app", "main");
        assert!(out.contains("[Image: Logo](https://github.com/octo/app/raw/main/docs/logo.png)"));
        assert!(out.contains("# Title"));
        assert!(!out.contains("hidden"));
        assert!(out.contains("[setup](https://github.com/octo/app/blob/main/docs/setup.md)"));
        assert!(out.contains("[site](https://x.dev)"));
        assert!(out.contains("[Image: Screen](https://github.com/octo/app/raw/main/docs/shot.png)"));
        assert!(out.contains("<div>[keep](as-is)</div>"));
        assert!(!out.contains("<p"));
    }

    #[test]
    fn keeps_code_and_autolinks_converts_html() {
        let out = prepare("Returns `Vec<String>` or `a[i](x)`, see <https://x.dev>.\n", "o/r", "main");
        assert_eq!(out, "Returns `Vec<String>` or `a[i](x)`, see <https://x.dev>.\n");
        let out = prepare("<h2 align=\"center\">Hello</h2>\n<a href='https://x.dev'>site</a> and <a href=\"docs/a.md\"><img src=\"b.svg\" alt=\"badge\"></a><br>\n", "o/r", "main");
        assert!(out.contains("## Hello"), "{out}");
        assert!(out.contains("[site](https://x.dev)"), "{out}");
        assert!(out.contains("[badge](https://github.com/o/r/blob/main/docs/a.md)"), "{out}");
        let out = prepare("[docs]: ./docs/setup.md \"Setup\"\n[web]: https://x.dev\n", "o/r", "main");
        assert_eq!(out, "[docs]: https://github.com/o/r/blob/main/docs/setup.md \"Setup\"\n[web]: https://x.dev\n");
    }
}
