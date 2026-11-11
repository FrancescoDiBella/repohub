//! 16-point line icons drawn for this app (stroke = currentColor).
#![allow(dead_code)]
use rune::Icon;

macro_rules! icon {
    ($id:ident, $name:literal, $body:literal) => {
        pub static $id: Icon = Icon { name: $name, view: 16.0, body: $body };
    };
}


icon!(HOME, "rh-home", r#"<path d="M2.5 7.5L8 3L13.5 7.5V13H9.75V9.75H6.25V13H2.5Z" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" fill="none"/>"#);
icon!(REPO, "rh-repo", r#"<path d="M3.5 3.25A1.25 1.25 0 0 1 4.75 2H12.5V11.5H4.75A1.25 1.25 0 0 0 3.5 12.75Z M3.5 12.75A1.25 1.25 0 0 0 4.75 14H12.5V11.5" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" fill="none"/>"#);
icon!(PULL, "rh-pull", r#"<circle cx="4.5" cy="3.75" r="1.5" stroke="currentColor" stroke-width="1.2" fill="none"/><circle cx="4.5" cy="12.25" r="1.5" stroke="currentColor" stroke-width="1.2" fill="none"/><circle cx="11.5" cy="12.25" r="1.5" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M4.5 5.25V10.75M11.5 10.75V6.5A2 2 0 0 0 9.5 4.5H7.5M8.75 3L7.25 4.5L8.75 6" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#);
icon!(ISSUE, "rh-issue", r#"<circle cx="8" cy="8" r="5.75" stroke="currentColor" stroke-width="1.2" fill="none"/><circle cx="8" cy="8" r="1.4" fill="currentColor"/>"#);
icon!(PLAY, "rh-play", r#"<circle cx="8" cy="8" r="5.75" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M6.75 5.75L10.5 8L6.75 10.25Z" fill="currentColor"/>"#);
icon!(GEAR, "rh-gear", r#"<circle cx="8" cy="8" r="2" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M8 1.75V3.5M8 12.5V14.25M1.75 8H3.5M12.5 8H14.25M3.6 3.6L4.8 4.8M11.2 11.2L12.4 12.4M3.6 12.4L4.8 11.2M11.2 4.8L12.4 3.6" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/>"#);
icon!(PLUS, "rh-plus", r#"<path d="M8 3.5V12.5M3.5 8H12.5" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/>"#);
icon!(STAR, "rh-star", r#"<path d="M8 2.25L9.7 5.9L13.6 6.3L10.65 8.95L11.5 12.85L8 10.85L4.5 12.85L5.35 8.95L2.4 6.3L6.3 5.9Z" stroke="currentColor" stroke-width="1.15" stroke-linejoin="round" fill="none"/>"#);
icon!(STAR_ON, "rh-star-on", r#"<path d="M8 2.25L9.7 5.9L13.6 6.3L10.65 8.95L11.5 12.85L8 10.85L4.5 12.85L5.35 8.95L2.4 6.3L6.3 5.9Z" fill="currentColor"/>"#);
icon!(FORK, "rh-fork", r#"<circle cx="4.5" cy="3.5" r="1.5" stroke="currentColor" stroke-width="1.2" fill="none"/><circle cx="11.5" cy="3.5" r="1.5" stroke="currentColor" stroke-width="1.2" fill="none"/><circle cx="8" cy="12.5" r="1.5" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M4.5 5V6.5A1.5 1.5 0 0 0 6 8H10A1.5 1.5 0 0 0 11.5 6.5V5M8 8V11" stroke="currentColor" stroke-width="1.2" fill="none"/>"#);
icon!(BRANCH, "rh-branch", r#"<circle cx="4.5" cy="3.5" r="1.5" stroke="currentColor" stroke-width="1.2" fill="none"/><circle cx="4.5" cy="12.5" r="1.5" stroke="currentColor" stroke-width="1.2" fill="none"/><circle cx="11.5" cy="5" r="1.5" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M4.5 5V11M11.5 6.5C11.5 9 8 9 5 11" stroke="currentColor" stroke-width="1.2" fill="none"/>"#);
icon!(FOLDER, "rh-folder", r#"<path d="M2 4.25A1.25 1.25 0 0 1 3.25 3H6.25L7.75 4.5H12.75A1.25 1.25 0 0 1 14 5.75V11.75A1.25 1.25 0 0 1 12.75 13H3.25A1.25 1.25 0 0 1 2 11.75Z" fill="currentColor"/>"#);
icon!(FILE, "rh-file", r#"<path d="M4 2H9.5L12.5 5V14H4Z M9.5 2V5H12.5" stroke="currentColor" stroke-width="1.15" stroke-linejoin="round" fill="none"/>"#);
icon!(CHECK, "rh-check", r#"<path d="M3.5 8.5L6.5 11.5L12.5 4.5" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#);
icon!(CROSS, "rh-cross", r#"<path d="M4.5 4.5L11.5 11.5M11.5 4.5L4.5 11.5" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/>"#);
icon!(DOT, "rh-dot", r#"<circle cx="8" cy="8" r="3.5" stroke="currentColor" stroke-width="1.6" fill="none" stroke-dasharray="3 2"/>"#);
icon!(COMMENT, "rh-comment", r#"<path d="M2.5 3.5H13.5V10.5H7L4 13V10.5H2.5Z" stroke="currentColor" stroke-width="1.15" stroke-linejoin="round" fill="none"/>"#);
icon!(GLOBE, "rh-globe", r#"<circle cx="8" cy="8" r="5.75" stroke="currentColor" stroke-width="1.15" fill="none"/><path d="M2.25 8H13.75M8 2.25C9.8 4 9.8 12 8 13.75M8 2.25C6.2 4 6.2 12 8 13.75" stroke="currentColor" stroke-width="1.15" fill="none"/>"#);
icon!(LOCK, "rh-lock", r#"<rect x="3.5" y="7" width="9" height="6.5" rx="1.25" stroke="currentColor" stroke-width="1.15" fill="none"/><path d="M5.5 7V5.25A2.5 2.5 0 0 1 10.5 5.25V7" stroke="currentColor" stroke-width="1.15" fill="none"/>"#);
icon!(BELL, "rh-bell", r#"<path d="M4 11V7.25A4 4 0 0 1 12 7.25V11L13 12.25H3Z M6.75 13.75H9.25" stroke="currentColor" stroke-width="1.15" stroke-linejoin="round" stroke-linecap="round" fill="none"/>"#);
icon!(BOOK, "rh-book", r#"<path d="M2.5 3.5C4.5 3 6.5 3.25 8 4.5C9.5 3.25 11.5 3 13.5 3.5V12.5C11.5 12 9.5 12.25 8 13.5C6.5 12.25 4.5 12 2.5 12.5Z M8 4.5V13.5" stroke="currentColor" stroke-width="1.15" stroke-linejoin="round" fill="none"/>"#);
icon!(MERGE, "rh-merge", r#"<circle cx="4.5" cy="3.5" r="1.5" stroke="currentColor" stroke-width="1.2" fill="none"/><circle cx="4.5" cy="12.5" r="1.5" stroke="currentColor" stroke-width="1.2" fill="none"/><circle cx="11.5" cy="9" r="1.5" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M4.5 5V11M4.5 5C5 7.5 7.5 9 10 9" stroke="currentColor" stroke-width="1.2" fill="none"/>"#);
icon!(BACK, "rh-back", r#"<path d="M9.5 3.5L5 8L9.5 12.5" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#);
icon!(MOON, "rh-moon", r#"<path d="M12.75 9.75A5.25 5.25 0 0 1 6.25 3.25A5.25 5.25 0 1 0 12.75 9.75Z" stroke="currentColor" stroke-width="1.15" stroke-linejoin="round" fill="none"/>"#);
icon!(REFRESH, "rh-refresh", r#"<path d="M12.75 6.5A5 5 0 1 0 13 9.5M12.75 2.75V6.5H9" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#);
icon!(EXTERNAL, "rh-external", r#"<path d="M9 3H13V7M13 3L7.5 8.5M11.5 9.5V12.5A.75.75 0 0 1 10.75 13.25H3.75A.75.75 0 0 1 3 12.5V5.5A.75.75 0 0 1 3.75 4.75H6.5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#);
icon!(PEN, "rh-pen", r#"<path d="M10.5 3L13 5.5L6 12.5L2.75 13.25L3.5 10Z M9 4.5L11.5 7" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" fill="none"/>"#);
icon!(TRASH, "rh-trash", r#"<path d="M3 4.5H13M6.25 4.5V3H9.75V4.5M4.5 4.5L5.25 13.25H10.75L11.5 4.5" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" stroke-linecap="round" fill="none"/>"#);
icon!(CURSOR, "rh-cursor", r#"<path d="M4 2.5L12 8.25L8.25 8.75L10.25 13L8.75 13.75L6.75 9.5L4 12Z" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" fill="none"/>"#);
icon!(SQUARE, "rh-square", r#"<rect x="3" y="3.5" width="10" height="9" rx="1.5" stroke="currentColor" stroke-width="1.3" fill="none"/>"#);
icon!(CIRCLE, "rh-circle", r#"<ellipse cx="8" cy="8" rx="5.25" ry="4.75" stroke="currentColor" stroke-width="1.3" fill="none"/>"#);
icon!(DIAMOND, "rh-diamond", r#"<path d="M8 2.5L13.5 8L8 13.5L2.5 8Z" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round" fill="none"/>"#);
icon!(ARROW, "rh-arrow", r#"<path d="M3 13L13 3M13 3H7.5M13 3V8.5" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#);
icon!(LINE, "rh-line", r#"<path d="M3 13L13 3" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/>"#);
icon!(TEXT, "rh-text", r#"<path d="M3.5 4V3H12.5V4M8 3V13M6 13H10" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#);
icon!(FILL, "rh-fill", r#"<rect x="3" y="3.5" width="10" height="9" rx="1.5" fill="currentColor" opacity=".35"/><rect x="3" y="3.5" width="10" height="9" rx="1.5" stroke="currentColor" stroke-width="1.3" fill="none"/>"#);
icon!(BOLD, "rh-bold", r#"<path d="M3 12.5C6 9.5 10 6 13 3.5" stroke="currentColor" stroke-width="3" stroke-linecap="round" fill="none"/>"#);
icon!(UNDO, "rh-undo", r#"<path d="M5.5 4L2.75 6.75L5.5 9.5M3 6.75H9.5A3.25 3.25 0 0 1 9.5 13.25H6.5" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#);
icon!(REDO, "rh-redo", r#"<path d="M10.5 4L13.25 6.75L10.5 9.5M13 6.75H6.5A3.25 3.25 0 0 0 6.5 13.25H9.5" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#);
icon!(TERMINAL, "rh-terminal", r#"<rect x="2" y="3" width="12" height="10" rx="1.75" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M4.75 6.25L6.75 8L4.75 9.75M8.25 10H11" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#);
icon!(SPARK, "rh-spark", r#"<path d="M8 2L9.4 6.6L14 8L9.4 9.4L8 14L6.6 9.4L2 8L6.6 6.6Z" fill="currentColor"/>"#);
icon!(SUN, "rh-sun", r#"<circle cx="8" cy="8" r="2.75" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M8 1.5V3M8 13V14.5M1.5 8H3M13 8H14.5M3.4 3.4L4.45 4.45M11.55 11.55L12.6 12.6M3.4 12.6L4.45 11.55M11.55 4.45L12.6 3.4" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/>"#);
icon!(SORT, "rh-sort", r#"<path d="M4.5 3V13M4.5 13L2.5 11M4.5 13L6.5 11M11.5 13V3M11.5 3L9.5 5M11.5 3L13.5 5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#);
icon!(COPY, "rh-copy", r#"<rect x="5.5" y="5.5" width="8" height="8" rx="1.5" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M10.5 3.5V3.25A.75.75 0 0 0 9.75 2.5H3.25A.75.75 0 0 0 2.5 3.25V9.75A.75.75 0 0 0 3.25 10.5H3.5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" fill="none"/>"#);
icon!(PIN, "rh-pin", r#"<path d="M6 2.5H10M6.75 2.5V6.5L4.5 9H11.5L9.25 6.5V2.5M8 9V13.5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#);
icon!(PIN_ON, "rh-pin-on", r#"<path d="M6 2.5H10M6.75 2.5V6.5L4.5 9H11.5L9.25 6.5V2.5Z" fill="currentColor" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round"/><path d="M8 9V13.5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/>"#);
