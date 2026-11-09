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
        let l = strip_tags(&l);
        let l = rewrite_links(&l, &blob, &raw);
        // Lines that were only HTML collapse to nothing; keep paragraph breaks single.
        if l.trim().is_empty() && line.trim().starts_with('<') {
            continue;
        }
        out.push_str(&l);
        out.push('\n');
    }
    out
}

/// Removes HTML tags, keeping the text between them. An `<img alt="x" src="y">` becomes a link.
fn strip_tags(l: &str) -> String {
    let mut out = String::new();
    let mut rest = l;
    while let Some(i) = rest.find('<') {
        let tail = &rest[i + 1..];
        let is_tag = tail.starts_with(|c: char| c.is_ascii_alphabetic() || c == '/' || c == '!');
        let Some(j) = tail.find('>').filter(|_| is_tag) else {
            out.push_str(&rest[..=i]);
            rest = &rest[i + 1..];
            continue;
        };
        out.push_str(&rest[..i]);
        let tag = &tail[..j];
        if tag.to_ascii_lowercase().starts_with("img") {
            let attr = |name: &str| {
                let k = format!("{name}=\"");
                tag.find(&k).and_then(|a| tag[a + k.len()..].split('"').next()).map(str::to_owned)
            };
            if let Some(src) = attr("src") {
                let alt = attr("alt").filter(|a| !a.is_empty()).unwrap_or_else(|| "image".into());
                out.push_str(&format!("![{alt}]({src})"));
            }
        }
        rest = &tail[j + 1..];
    }
    out.push_str(rest);
    out
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
        let absolute = |base: &str| {
            if url.starts_with("http://") || url.starts_with("https://") || url.starts_with("mailto:") || url.starts_with('#') {
                url.to_owned()
            } else {
                format!("{base}{}", url.trim_start_matches("./").trim_start_matches('/'))
            }
        };
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
}
