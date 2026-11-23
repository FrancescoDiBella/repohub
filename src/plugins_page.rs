//! Settings → Plugins: the installed plugins (on/off, reload, remove), installing one from a
//! folder or a Git address, and starting your own from the example.
use rune::{El, col, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit};

use crate::ui::{self, panel, panel_head, rule};
use crate::{App, icons};

const GUIDE: &str = "https://github.com/FrancescoDiBella/repohub/blob/main/docs/PLUGINS.md";

pub fn plugins_panel(s: &App, k: &Kit) -> El {
    let guide = Button::new("Guide").icon(&icons::EXTERNAL).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
        .on_press_tx(|_: &mut App, tx| { let _ = tx.open_url(GUIDE); }).build(k).key("plugins-guide");
    let p = panel(k).key("plugins").child(panel_head(k, "Plugins", Some(guide))).child(rule(k));
    if cfg!(target_arch = "wasm32") {
        return p.child(col().w_full().padding(16.0).child(text("Plugins run in the desktop app, which can read your plugins folder.")
            .font(k.ty.small).color(k.c.text_muted).wrap()));
    }
    let mut body = col().w_full().padding(16.0).gap(12.0)
        .child(text("Plugins add a section to repository pages or a page of their own. They are small Luau scripts, written by whoever made them, and they run sandboxed: no files, no programs, no network.")
            .font(k.ty.text_small).color(k.c.text_muted).wrap())
        .child(text("A plugin can read what RepoHub shows (your repositories, issues and pull requests, private ones too) and show notifications. To open a link or copy text it has to ask you each time: a link could carry what it read. Install only plugins you trust.")
            .font(k.ty.text_small).color(k.c.text_muted).wrap());
    if s.plugins.list.is_empty() {
        body = body.child(text("No plugins yet.").font(k.ty.small).color(k.c.text_faint));
    }
    for p in &s.plugins.list {
        body = body.child(plugin_row(s, k, p));
    }
    p.child(body.child(install_row(s, k)))
}

fn plugin_row(s: &App, k: &Kit, p: &crate::plugins::Plugin) -> El {
    let m = &p.manifest;
    let id = p.id.clone();
    let mut places = Vec::new();
    if let Some(t) = &m.repo_tab {
        places.push(format!("section “{t}”"));
    }
    if let Some(t) = &m.page {
        places.push(format!("page “{t}”"));
    }
    let meta = [m.version.clone(), if m.author.is_empty() { String::new() } else { format!("by {}", m.author) }, places.join(", ")]
        .into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · ");
    let mut info = col().grow(1.0).shrink().min_w(220.0).gap(2.0)
        .child(row().gap(8.0).center_y()
            .child(ui::ico(&icons::SPARK, 14.0, if p.running() { k.c.text_accent } else { k.c.icon_faint }))
            .child(text(m.name.clone()).font(k.ty.ui_medium).color(k.c.text_base)))
        .child(text(meta).font(k.ty.text_small).color(k.c.text_muted).wrap());
    if !m.description.is_empty() {
        info = info.child(text(m.description.clone()).font(k.ty.text_small).color(k.c.text_base).wrap());
    }
    if let Some(t) = p.trouble() {
        info = info.child(text(t).font(k.ty.text_small).color(k.c.danger.icon).wrap());
    }
    #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
    let mut actions = row().gap(6.0).center_y().flex_wrap();
    #[cfg(not(target_arch = "wasm32"))]
    {
        let (on, reload, remove) = (id.clone(), id.clone(), id.clone());
        let folder = format!("file://{}", p.dir.display().to_string().replace('%', "%25").replace(' ', "%20").replace('\\', "/"));
        actions = actions
            .child(Button::new("Reload").variant(ButtonVariant::Ghost).control_size(ControlSize::Small).disabled(!p.enabled)
                .on_press(move |s: &mut App| if let Some(h) = s.handle.clone() { s.plugins.reload(&reload, &h) }).build(k).key(format!("plugin-reload-{id}")))
            .child(Button::new("Folder").icon(&icons::FOLDER).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
                .on_press_tx(move |_: &mut App, tx| { let _ = tx.open_url(&folder); }).build(k).key(format!("plugin-folder-{id}")))
            .child(Button::new("Remove").variant(ButtonVariant::Danger).control_size(ControlSize::Small)
                .on_press(move |s: &mut App| s.ask(format!("ask-remove-plugin-{remove}"))).build(k).key(format!("plugin-remove-{id}")))
            .child(rune_kit::Switch::new(p.enabled).label(format!("{} on", m.name))
                .on_toggle(move |s: &mut App| {
                    let now = !s.plugins.list.iter().any(|p| p.id == on && p.enabled);
                    let h = s.handle.clone();
                    s.plugins.set_enabled(&on, now, h.as_ref());
                    s.plugin_tab = None;
                }).build(k).key(format!("plugin-on-{id}")));
    }
    let _ = (s, &id);
    // Removing asks first: its folder is deleted.
    #[cfg(not(target_arch = "wasm32"))]
    let question = {
        let gone = id.clone();
        crate::confirm::Confirm::new(format!("ask-remove-plugin-{id}"), &icons::TRASH, format!("Remove the plugin {}?", m.name))
            .detail(format!("Its folder ({}) is deleted from this computer.", p.dir.display()))
            .yes("Remove", crate::confirm::Tone::Danger, move |s: &mut App| s.remove_plugin(&gone))
            .show(s, k)
    };
    #[cfg(target_arch = "wasm32")]
    let question: Option<El> = None;
    let line = row().w_full().gap(12.0).py(8.0).flex_wrap().center_y().key(format!("plugin-{}", p.id)).child(info);
    match question {
        Some(q) => col().w_full().gap(8.0).pb(8.0).child(line).child(q),
        None => line.child(actions),
    }
}

fn install_row(s: &App, k: &Kit) -> El {
    #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
    let mut out = col().w_full().gap(8.0).pt(4.0).child(rule(k));
    #[cfg(not(target_arch = "wasm32"))]
    {
        let input = rune_kit::TextInput::new(&s.plugin_src).label("Plugin folder or Git address")
            .placeholder("A folder with plugin.json, or https://github.com/you/plugin")
            .on_edit(|s: &mut App| &mut s.plugin_src).on_submit(|s: &mut App, _| s.install_plugin(false)).build(k).key("plugin-src").grow(1.0).min_w(260.0);
        let dir = s.plugins_folder.clone();
        let display_dir = dir.display().to_string();
        let folder = format!("file://{}", display_dir.replace('%', "%25").replace(' ', "%20").replace('\\', "/"));
        out = out
            .child(row().w_full().gap(8.0).pt(8.0).center_y().flex_wrap()
                .child(input)
                .child(Button::new(if s.plugin_installing { "Installing…" } else { "Install" }).icon(&icons::PLUS).control_size(ControlSize::Small)
                    .disabled(s.plugin_installing || s.plugin_src.text.trim().is_empty())
                    .on_press(|s: &mut App| s.install_plugin(false)).build(k).key("plugin-install")))
            .child(row().w_full().gap(8.0).center_y().flex_wrap()
                .child(Button::new("Create a plugin").icon(&icons::SPARK).variant(ButtonVariant::Submit).control_size(ControlSize::Small)
                    .help("Writes the example plugin into your plugins folder, to change as you like")
                    .on_press(|s: &mut App| s.install_plugin(true)).build(k).key("plugin-create"))
                .child(Button::new("Open plugins folder").icon(&icons::FOLDER).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
                    .on_press_tx(move |_: &mut App, tx| {
                        let _ = std::fs::create_dir_all(&dir);
                        let _ = tx.open_url(&folder);
                    }).build(k).key("plugins-folder"))
                .child(text(display_dir).font(k.ty.text_small).color(k.c.text_faint).shrink().max_lines(1)));
        if let Some(e) = &s.plugin_error {
            out = out.child(text(e.clone()).font(k.ty.small).color(k.c.danger.icon).wrap());
        }
    }
    let _ = s;
    out
}
