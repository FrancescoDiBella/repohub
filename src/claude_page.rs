//! The Claude tab: a chat with Claude Code in the repository's local clone, like the Claude Code
//! extension for VS Code. The engine is in `claude.rs`.
use rune::{El, ScrollSpec, col, el, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit, SegmentedControl, TextArea};

use crate::claude::{Item, Mode, Run};
use crate::ui::{self, panel};
use crate::{App, clones_shim, icons, terminal_page};

/// Prompts offered on an empty chat.
const STARTERS: [&str; 4] = [
    "Explain how this repository is organised",
    "Review my uncommitted changes",
    "Find bugs in the last few commits",
    "Suggest tests for the most important code",
];

fn web_only(k: &Kit) -> El {
    panel(k).padding(24.0).gap(10.0)
        .child(row().gap(10.0).center_y().child(ui::ico(&icons::SPARK, 18.0, k.c.icon_muted))
            .child(text("Claude needs the desktop app").font(k.ty.body_medium).color(k.c.text_strong)))
        .child(text("The Claude tab runs your own Claude Code in the repository's folder on your computer, which a web page can't do. Open RepoHub as the desktop app.")
            .font(k.ty.ui).color(k.c.text_muted).wrap())
}

pub fn claude_tab(s: &App, k: &Kit, i: usize) -> El {
    if cfg!(target_arch = "wasm32") {
        return web_only(k);
    }
    let r = &s.data.repos[i];
    let Some(dir) = clones_shim::find(&r.full_name) else {
        return col().w_full().gap(10.0)
            .child(panel(k).padding(20.0).gap(8.0)
                .child(row().gap(10.0).center_y().child(ui::ico(&icons::SPARK, 18.0, k.c.icon_muted))
                    .child(text("Claude works in your local copy").font(k.ty.body_medium).color(k.c.text_strong)))
                .child(text("Claude Code reads and edits the repository's files on this computer. Pick the folder of your local copy, or clone the repository.")
                    .font(k.ty.small).color(k.c.text_muted).wrap()))
            .child(terminal_page::pick_folder(s, k, i, false));
    };
    let chat = s.chats.get(&r.full_name);
    let run = chat.map(|c| c.run).unwrap_or_default();
    let busy = run.is_busy();
    // Head: where Claude works, what it may do, and a fresh start.
    let status = match run {
        Run::Idle => None,
        Run::Starting => Some(ui::pill(k, "Starting", k.c.text_muted)),
        Run::Running => Some(ui::pill(k, "Working", k.c.info.icon)),
        Run::Stopping => Some(ui::pill(k, "Stopping", k.c.warning.icon)),
    };
    let mode = SegmentedControl::new(&Mode::LABELS, Some(s.claude_mode.index())).label("What Claude may do").fit()
        .on_pick(|s: &mut App, m, _| s.claude_mode = Mode::ALL[m.min(1)]).build(k).key("claude-mode");
    let new_chat = Button::new("New chat").icon(&icons::PLUS).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
        .disabled(busy || chat.is_none_or(|c| c.items.is_empty()))
        .on_press(move |s: &mut App| s.new_chat(i)).build(k).key("claude-new");
    let mut head = row().w_full().gap(10.0).center_y().flex_wrap()
        .child(ui::ico(&icons::FOLDER, 14.0, k.c.info.icon))
        .child(text(dir.display().to_string()).font(k.ty.mono).color(k.c.text_base).grow(1.0).shrink().max_lines(1).min_w(160.0));
    if let Some(p) = status {
        head = head.child(p);
    }
    let head = head.child(mode).child(new_chat);
    // The conversation.
    let mut list = col().w_full().gap(14.0);
    match chat.filter(|c| !c.items.is_empty()) {
        None => {
            let mut starters = row().gap(8.0).flex_wrap();
            for (n, p) in STARTERS.iter().enumerate() {
                let prompt = (*p).to_owned();
                starters = starters.child(Button::new(*p).icon(&icons::SPARK).control_size(ControlSize::Small)
                    .on_press(move |s: &mut App| s.ask_claude(i, prompt.clone())).build(k).key(format!("starter-{n}")));
            }
            list = list.child(col().w_full().gap(10.0).padding(8.0)
                .child(text("Ask Claude about this repository").font(k.ty.body_medium).color(k.c.text_strong))
                .child(text("Claude Code runs in the folder above with your own login. In Plan it only reads; with Edit files it can change files in your local copy. Commands it isn't allowed to run are listed after its answer.")
                    .font(k.ty.small).color(k.c.text_muted).wrap())
                .child(starters));
        }
        Some(c) => {
            for (n, item) in c.items.iter().enumerate() {
                list = list.child(item_view(k, n, item));
            }
            if busy && !matches!(c.items.last(), Some(Item::Done { .. })) {
                list = list.child(row().gap(8.0).center_y()
                    .child(ui::ico(&icons::DOT, 14.0, k.c.info.icon))
                    .child(text(if run == Run::Stopping { "Stopping…" } else { "Claude is working…" }).font(k.ty.small).color(k.c.text_muted)));
            }
        }
    }
    let convo = col().key("claude-chat").w_full().max_h(560.0).padding(16.0).radius(k.radius.lg).fill(k.c.bg_base).border(1.0, k.c.border_weak)
        .scroll(ScrollSpec { stick_bottom: true }).child(list);
    // The composer: Enter sends, Shift+Enter starts a new line.
    let field = TextArea::new(&s.claude_prompt).placeholder(if chat.is_some_and(|c| c.session.is_some()) { "Reply to Claude" } else { "Ask Claude to explain, review or change something" })
        .label("Message to Claude").auto_grow(2, 8)
        .on_edit(|s: &mut App| &mut s.claude_prompt)
        .on_submit(move |s: &mut App, _| { let p = s.claude_prompt.text.clone(); s.ask_claude(i, p); })
        .build(k).key("claude-input").grow(1.0).min_w(0.0);
    let action = if busy {
        Button::new("Stop").icon(&icons::CROSS).variant(ButtonVariant::Danger).control_size(ControlSize::Small)
            .disabled(run == Run::Stopping)
            .on_press(move |s: &mut App| s.stop_claude(i)).build(k).key("claude-stop")
    } else {
        Button::new("Send").icon(&icons::SPARK).variant(ButtonVariant::Submit).control_size(ControlSize::Small)
            .disabled(s.claude_prompt.text.trim().is_empty())
            .on_press(move |s: &mut App| { let p = s.claude_prompt.text.clone(); s.ask_claude(i, p); }).build(k).key("claude-send")
    };
    let foot = match chat {
        Some(c) if c.cost() > 0.0 => format!("This chat: ${:.2}{}", c.cost(), c.model.as_ref().map(|m| format!(" · {m}")).unwrap_or_default()),
        _ => "Enter sends, Shift+Enter adds a line.".into(),
    };
    col().w_full().gap(10.0)
        .child(head)
        .child(convo)
        .child(row().w_full().gap(8.0).items_end().child(field).child(action))
        .child(text(foot).font(k.ty.text_small).color(k.c.text_faint))
}

fn tool_icon(name: &str) -> &'static rune::Icon {
    match name {
        "Bash" => &icons::TERMINAL,
        "Read" | "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => &icons::FILE,
        "Grep" | "Glob" => &icons::BOOK,
        "WebFetch" | "WebSearch" => &icons::GLOBE,
        "TodoWrite" => &icons::CHECK,
        _ => &icons::SPARK,
    }
}

fn item_view(k: &Kit, n: usize, item: &Item) -> El {
    match item {
        Item::User(t) => row().w_full().justify_end()
            .child(text(t.clone()).font(k.ty.ui).color(k.c.text_base).wrap().selectable(true)
                .px(12.0).py(8.0).radius(k.radius.lg).fill(k.c.bg_accent.alpha(0.12)).max_w(620.0)),
        Item::Text(t) => col().w_full().key(format!("claude-text-{n}")).child(crate::pages::readme_view(k, t)),
        Item::Tool { name, summary, result, error, .. } => {
            let ink = if *error { k.c.danger.icon } else { k.c.icon_muted };
            let mut c = col().w_full().gap(4.0).px(10.0).py(8.0).radius(k.radius.md).fill(k.c.layer_01)
                .child(row().w_full().gap(8.0).center_y()
                    .child(ui::ico(tool_icon(name), 14.0, ink))
                    .child(text(name.clone()).font(k.ty.small_medium).color(k.c.text_base))
                    .child(text(summary.clone()).font(k.ty.mono).color(k.c.text_muted).grow(1.0).shrink().max_lines(1))
                    .child(if result.is_none() { ui::pill(k, "running", k.c.text_faint) } else { el() }));
            if let Some(out) = result.as_ref().filter(|o| !o.trim().is_empty()) {
                let lines: Vec<&str> = out.lines().take(4).collect();
                let more = out.lines().count().saturating_sub(4);
                let mut shown = lines.join("\n");
                if more > 0 {
                    shown.push_str(&format!("\n… {more} more lines"));
                }
                c = c.child(text(shown).font(k.ty.mono).color(if *error { k.c.danger.icon } else { k.c.text_faint }).wrap().selectable(true));
            }
            c
        }
        Item::Done { seconds, cost, turns, error, denied } => {
            let mut c = col().w_full().gap(6.0);
            if let Some(e) = error {
                c = c.child(text(e.clone()).font(k.ty.small).color(k.c.danger.icon).wrap());
            }
            if !denied.is_empty() {
                c = c.child(col().w_full().gap(4.0).px(10.0).py(8.0).radius(k.radius.md).border(1.0, k.c.warning.icon.alpha(0.5))
                    .child(text("Not allowed in this mode (switch to Edit files, or run it in the Terminal tab):").font(k.ty.small_medium).color(k.c.text_base).wrap())
                    .children(denied.iter().map(|d| text(d.clone()).font(k.ty.mono).color(k.c.text_muted).wrap())));
            }
            c.child(text(format!("{:.1} s · ${cost:.2} · {turns} {}", seconds, if *turns == 1 { "turn" } else { "turns" })).font(k.ty.text_small).color(k.c.text_faint))
        }
        Item::Problem(p) => row().w_full().gap(8.0).px(10.0).py(8.0).radius(k.radius.md).border(1.0, k.c.danger.icon.alpha(0.5))
            .child(ui::ico(&icons::CROSS, 14.0, k.c.danger.icon))
            .child(text(p.clone()).font(k.ty.small).color(k.c.text_base).wrap().grow(1.0).shrink().selectable(true)),
    }
}
