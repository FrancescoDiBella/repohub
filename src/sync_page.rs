//! What board sync shows: the Notes section's sync line, the cloud mark on each board, and the
//! Boards block in Settings → Account.
use rune::{El, Icon, col, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit, TextInput};

use crate::state::{BoardSync, NotesRepoCheck};
use crate::sync::Key;
use crate::ui::{self, panel, panel_head, rule};
use crate::{App, icons, sync_io};

pub static CLOUD_DONE: Icon = Icon { name: "rh-cloud-done", view: 16.0, body: r#"<path d="M4.75 12.75H11.5A2.75 2.75 0 0 0 11.9 7.28A4 4 0 0 0 4.2 7.6A2.6 2.6 0 0 0 4.75 12.75Z" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" fill="none"/><path d="M6.4 9.9L7.6 11L9.7 8.7" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"# };
pub static CLOUD_UP: Icon = Icon { name: "rh-cloud-up", view: 16.0, body: r#"<path d="M4.75 12.75H11.5A2.75 2.75 0 0 0 11.9 7.28A4 4 0 0 0 4.2 7.6A2.6 2.6 0 0 0 4.75 12.75Z" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" fill="none"/><path d="M8 11.4V8.3M6.7 9.5L8 8.2L9.3 9.5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"# };
pub static CLOUD_ALERT: Icon = Icon { name: "rh-cloud-alert", view: 16.0, body: r#"<path d="M4.75 12.75H11.5A2.75 2.75 0 0 0 11.9 7.28A4 4 0 0 0 4.2 7.6A2.6 2.6 0 0 0 4.75 12.75Z" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" fill="none"/><path d="M8 8.1V10" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/><circle cx="8" cy="11.4" r="0.7" fill="currentColor"/>"# };
pub static CLOUD_OFF: Icon = Icon { name: "rh-cloud-off", view: 16.0, body: r#"<path d="M4.75 12.75H11.5A2.75 2.75 0 0 0 11.9 7.28A4 4 0 0 0 4.2 7.6A2.6 2.6 0 0 0 4.75 12.75Z" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" fill="none"/><path d="M2.75 2.75L13.25 13.25" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/>"# };
pub static EXPORT: Icon = Icon { name: "rh-export", view: 16.0, body: r#"<path d="M8 2.5V10M5.25 5.25L8 2.5L10.75 5.25M3.5 9V12.75A.75.75 0 0 0 4.25 13.5H11.75A.75.75 0 0 0 12.5 12.75V9" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"# };
pub static IMPORT: Icon = Icon { name: "rh-import", view: 16.0, body: r#"<path d="M8 2.5V10M5.25 7.25L8 10L10.75 7.25M3.5 9V12.75A.75.75 0 0 0 4.25 13.5H11.75A.75.75 0 0 0 12.5 12.75V9" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"# };

/// "3 boards", "1 board".
fn boards(n: usize) -> String {
    if n == 1 { "1 board".into() } else { format!("{n} boards") }
}

/// Whether a board is the other device's copy kept by a conflict.
pub fn is_conflict_copy(name: &str) -> bool {
    name.contains(" (conflict ")
}

/// The Notes section's sync line: where boards sync, how the last sync went, and **Sync now**.
pub fn header(s: &App, k: &Kit) -> El {
    let line = |icon: &'static Icon, ink: rune::Color, msg: String| {
        row().gap(8.0).center_y().grow(1.0).shrink().min_w(0.0)
            .child(ui::ico(icon, 16.0, ink))
            .child(text(msg).font(k.ty.small).color(k.c.text_muted).wrap().shrink())
    };
    let bar = row().w_full().px(12.0).py(8.0).gap(10.0).center_y().flex_wrap().radius(k.radius.lg)
        .border(1.0, k.c.border_weak).key("sync-line");
    let Some(repo) = s.sync.repo.clone() else {
        let msg = match s.sync_blocked() {
            Some(why) => format!("Boards stay on this device. {why}"),
            None => "Boards stay on this device. To see them on your other devices, sync them through a private repository.".into(),
        };
        let mut bar = bar.child(line(&CLOUD_OFF, k.c.icon_muted, msg));
        if s.sync_blocked().is_none() {
            bar = bar.child(Button::new("Set up sync").variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
                .on_press(|s: &mut App| {
                    s.settings_section = crate::state::SettingsSection::Account;
                    s.go(crate::Page::Settings);
                }).build(k).key("sync-setup"));
        }
        return bar;
    };
    if let Some(why) = s.sync_blocked() {
        return bar.child(line(&CLOUD_OFF, k.c.icon_muted, format!("Sync is paused: {why}")));
    }
    let waiting = s.sync.to_send.len();
    let (icon, ink, msg) = match &s.sync.status {
        BoardSync::Off | BoardSync::Ready if waiting > 0 => (&CLOUD_UP, k.c.icon_muted, format!("{} to send to {repo}", boards(waiting))),
        BoardSync::Off | BoardSync::Ready => (&CLOUD_UP, k.c.icon_muted, format!("Not synced yet with {repo}")),
        BoardSync::Running => (&icons::REFRESH, k.c.text_accent, format!("Syncing with {repo}…")),
        BoardSync::Done { .. } if waiting > 0 => (&CLOUD_UP, k.c.icon_muted, format!("{} to send to {repo}", boards(waiting))),
        BoardSync::Done { at } => (&CLOUD_DONE, k.c.success.icon, format!("Synced {} with {repo}", crate::model::ago(*at, s.now.max(*at)))),
        BoardSync::Failed(e) if waiting > 0 => (&CLOUD_ALERT, k.c.danger.icon, format!("{} waiting to sync: {e}", boards(waiting))),
        BoardSync::Failed(e) => (&CLOUD_ALERT, k.c.danger.icon, format!("Not synced: {e}")),
    };
    let failed = matches!(s.sync.status, BoardSync::Failed(_));
    let mut bar = bar.child(line(icon, ink, msg));
    if failed {
        bar = bar.role("alert");
    }
    if s.sync.public {
        bar = bar.child(text(format!("{repo} is public: everyone can see these boards.")).font(k.ty.text_small).color(k.c.danger.icon).wrap());
    }
    if s.sync.partial {
        bar = bar.child(text("GitHub listed only part of the notes repository, so boards deleted there aren't deleted here.").font(k.ty.text_small).color(k.c.text_muted).wrap());
    }
    bar.child(Button::new(if failed { "Retry" } else { "Sync now" }).icon(&icons::REFRESH).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
        .disabled(s.sync.status == BoardSync::Running)
        .on_press(|s: &mut App| s.sync_now()).build(k).key("sync-now"))
}

/// The cloud mark on a board in the list: synced, waiting to sync, or a conflict copy. A word
/// goes with the icon (and is its label), never the colour alone.
pub fn mark(s: &App, k: &Kit, key: &Key) -> Option<El> {
    if s.sync.repo.is_none() || s.sync_blocked().is_some() {
        return None;
    }
    let (icon, word, ink) = if is_conflict_copy(&key.name) {
        (&CLOUD_ALERT, "Conflict copy", k.c.danger.icon)
    } else if s.sync.to_send.contains(key) {
        (&CLOUD_UP, "Waiting to sync", k.c.text_muted)
    } else if s.sync.synced.contains(key) {
        (&CLOUD_DONE, "Synced", k.c.success.icon)
    } else {
        (&CLOUD_UP, "Not synced yet", k.c.text_faint)
    };
    Some(row().gap(4.0).center_y().role("img").label(word.to_owned()).help(match word {
        "Conflict copy" => "Another device's copy that couldn't be merged with this board's: keep what you need, then delete it.".to_owned(),
        w => format!("{w} with {}", s.sync.repo.clone().unwrap_or_default()),
    })
        .child(ui::ico(icon, 14.0, ink))
        .child(text(word.to_owned()).font(k.ty.text_small).color(ink)))
}

/// Settings → Account → Boards: the notes repository, **Use it**, its state, **Stop syncing**.
pub fn settings_block(s: &App, k: &Kit) -> El {
    let create = crate::pages::open_button(k, "Create it on GitHub", &sync_io::create_url(&s.cfg), "notes-repo-create");
    let head = panel(k).key("boards-sync").child(panel_head(k, "Boards", if s.sync.repo.is_none() && s.sync_blocked().is_none() { Some(create) } else { None })).child(rule(k));
    let about = text("Boards sync through one private repository on your GitHub account, so a board drawn on one device opens on the others. RepoHub writes only to its boards/ folder (one commit per change); the token needs Contents: read and write there.")
        .font(k.ty.text_small).color(k.c.text_muted).wrap();
    let mut body = col().w_full().padding(16.0).gap(12.0).child(about);
    if let Some(why) = s.sync_blocked() {
        return head.child(body.child(row().gap(8.0).center_y().child(ui::ico(&CLOUD_OFF, 16.0, k.c.icon_muted))
            .child(text(why).font(k.ty.small).color(k.c.text_base).wrap().shrink())));
    }
    if let Some(repo) = s.sync.repo.clone() {
        let state = match &s.sync.status {
            BoardSync::Running => "Syncing…".to_owned(),
            BoardSync::Done { at } => format!("Synced {}", crate::model::ago(*at, s.now.max(*at))),
            BoardSync::Failed(e) => format!("Not synced: {e}"),
            BoardSync::Off | BoardSync::Ready => "Not synced yet".to_owned(),
        };
        let waiting = s.sync.to_send.len();
        let state = if waiting > 0 { format!("{state} · {} to send", boards(waiting)) } else { state };
        body = body.child(row().w_full().gap(10.0).center_y().flex_wrap()
            .child(ui::ico(if s.sync.public { &icons::GLOBE } else { &icons::LOCK }, 16.0, k.c.icon_muted))
            .child(col().grow(1.0).shrink().min_w(200.0).gap(2.0)
                .child(text(format!("Syncing through {repo}")).font(k.ty.ui_medium).color(k.c.text_base).wrap())
                .child(text(state).font(k.ty.text_small).color(if matches!(s.sync.status, BoardSync::Failed(_)) { k.c.danger.icon } else { k.c.text_muted }).wrap())));
        if s.sync.public {
            body = body.child(text(format!("{repo} is public: everyone can see your boards. Make it private on GitHub (Settings → Danger zone → Change visibility).")).font(k.ty.small).color(k.c.danger.icon).wrap());
        }
        body = body.child(row().gap(8.0).flex_wrap()
            .child(Button::new("Sync now").icon(&icons::REFRESH).control_size(ControlSize::Small).disabled(s.sync.status == BoardSync::Running)
                .on_press(|s: &mut App| s.sync_now()).build(k).key("settings-sync-now"))
            .child(Button::new("Stop syncing").variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
                .on_press(|s: &mut App| s.stop_syncing()).build(k).key("stop-syncing")))
            .child(text("Stop syncing leaves the boards here and in the repository as they are.").font(k.ty.text_small).color(k.c.text_faint).wrap());
        return head.child(body);
    }
    let checking = s.sync.check == NotesRepoCheck::Checking;
    let field = TextInput::new(&s.sync.field).placeholder(format!("{}/{}", if s.data.profile.login.is_empty() { "owner" } else { &s.data.profile.login }, sync_io::SUGGESTED))
        .label("Notes repository (owner/name)")
        .on_edit(|s: &mut App| &mut s.sync.field).on_submit(|s: &mut App, _| s.use_notes_repo(None)).build(k).key("notes-repo").grow(1.0).min_w(200.0);
    let use_it = Button::new(if checking { "Checking…" } else { "Use it" }).variant(ButtonVariant::Submit).control_size(ControlSize::Small)
        .disabled(checking || s.sync.field.text.trim().is_empty())
        .on_press(|s: &mut App| s.use_notes_repo(None)).build(k).key("notes-repo-use");
    body = body.child(row().w_full().gap(10.0).center_y().flex_wrap()
        .child(text("Notes repository").font(k.ty.small).color(k.c.text_muted).w(130.0))
        .child(field).child(use_it));
    match &s.sync.check {
        NotesRepoCheck::ConfirmPublic(repo) => {
            let r = repo.clone();
            body = body.child(col().w_full().gap(8.0).padding(12.0).radius(k.radius.md).border(1.0, k.c.danger.icon.alpha(0.6)).role("alert")
                .child(row().gap(8.0).center_y().child(ui::ico(&icons::GLOBE, 16.0, k.c.danger.icon))
                    .child(text(format!("{repo} is public. Everyone could see your boards.")).font(k.ty.ui_medium).color(k.c.danger.icon).wrap().shrink()))
                .child(text("Make it private on GitHub first, or go on if that's what you want.").font(k.ty.text_small).color(k.c.text_muted).wrap())
                .child(row().gap(8.0).flex_wrap()
                    .child(Button::new("Use it anyway").variant(ButtonVariant::Danger).control_size(ControlSize::Small)
                        .on_press(move |s: &mut App| s.use_notes_repo(Some(r.clone()))).build(k).key("notes-repo-public-ok"))
                    .child(Button::new("Cancel").variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
                        .on_press(|s: &mut App| s.sync.check = NotesRepoCheck::Idle).build(k).key("notes-repo-public-cancel"))));
        }
        NotesRepoCheck::Failed(e) => body = body.child(text(e.clone()).font(k.ty.small).color(k.c.danger.icon).wrap()),
        NotesRepoCheck::Checking => body = body.child(text("Checking the repository on GitHub…").font(k.ty.small).color(k.c.text_muted)),
        NotesRepoCheck::Idle => body = body.child(text(format!("Empty: sync is off. Create a private repository (for example {}), then use it here.", sync_io::SUGGESTED)).font(k.ty.text_small).color(k.c.text_faint).wrap()),
    }
    head.child(body)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use crate::state::{BoardSync, NotesRepoCheck, RepoTab, SettingsSection};
    use crate::sync::Key;
    use crate::{App, Page, notes};

    fn app() -> App {
        let mut s = crate::init();
        s.data = crate::model::Data::sample();
        s.pending_place = None;
        s
    }

    fn texts(rt: &rune::Runtime<App>) -> Vec<String> {
        rt.elements().iter().filter_map(|e| e.text.clone()).collect()
    }

    fn has(rt: &rune::Runtime<App>, needle: &str) -> bool {
        texts(rt).iter().any(|t| t.contains(needle))
    }

    fn press(rt: &mut rune::Runtime<App>, key: &str) {
        let (x, y) = rt.find(key).unwrap_or_else(|| panic!("{key} is on screen"));
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.1);
    }

    fn notes_page(s: &mut App) -> (String, String) {
        let full = s.data.repos[0].full_name.clone();
        let branch = s.notes_branch(0).unwrap().1;
        s.page = Page::Repo(full.clone());
        s.tab = RepoTab::Notes;
        let now = crate::net::now();
        let items = ["Plan", "Sketch", "Plan (conflict 2026-11-20)"].map(|n| notes::Note { name: n.into(), updated_at: now - 60, size: 2048 }).to_vec();
        s.notes.insert((full.clone(), branch.clone()), notes::NotesView { items, state: crate::state::Refresh::Fresh });
        (full, branch)
    }

    #[test]
    fn the_notes_header_says_how_sync_went_and_marks_each_board() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _dir = crate::load_gen_tests::TempDataDir::new("sync-header");
        let mut s = app();
        let (full, branch) = notes_page(&mut s);
        // Sample data: sync needs an account, and the line says so.
        let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 2400.0), rune::Env::new(true, 1.0));
        rt.frame(0.0);
        assert!(has(&rt, "Boards stay on this device. Sync needs your GitHub account"), "{:?}", texts(&rt));
        assert!(rt.find("sync-now").is_none());
        // Synced, one board waiting, a conflict copy.
        rt.update(|s, _| {
            s.sync_still("synced", Some("me/repohub-notes".into()));
            s.sync.to_send.insert(Key::new(&full, &branch, "Sketch"));
            s.sync.synced.remove(&Key::new(&full, &branch, "Sketch"));
        });
        rt.frame(0.1);
        assert!(has(&rt, "1 board to send to me/repohub-notes"), "{:?}", texts(&rt));
        let labels: Vec<String> = rt.elements().iter().filter_map(|e| e.label.clone()).collect();
        for word in ["Synced", "Waiting to sync", "Conflict copy"] {
            assert!(labels.iter().any(|l| l == word), "a mark says {word}: {labels:?}");
        }
        rt.update(|s, _| s.sync.to_send.clear());
        rt.frame(0.2);
        assert!(has(&rt, "Synced 2 minutes ago with me/repohub-notes"), "{:?}", texts(&rt));
        assert!(rt.find("sync-now").is_some() && rt.find("notes-import").is_some() && rt.find("export-note-Plan").is_some());
        // Offline: the error, what waits, and Retry.
        rt.update(|s, _| s.sync_still("failed", Some("me/repohub-notes".into())));
        rt.frame(0.3);
        assert!(has(&rt, "2 boards waiting to sync: Can't reach"), "{:?}", texts(&rt));
        assert!(texts(&rt).iter().any(|t| t == "Retry"));
        rt.update(|s, _| s.sync.status = BoardSync::Running);
        rt.frame(0.4);
        assert!(has(&rt, "Syncing with me/repohub-notes…"));
    }

    /// The Notes section's first line says where the drawings are: here only, or here and in
    /// the notes repository once sync is on.
    #[test]
    fn the_notes_section_says_the_boards_also_go_to_the_notes_repository() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _dir = crate::load_gen_tests::TempDataDir::new("sync-where");
        let mut s = app();
        notes_page(&mut s);
        let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 2400.0), rune::Env::new(true, 1.0));
        rt.frame(0.0);
        assert!(has(&rt, "for this repository, saved on this computer. Each branch"), "{:?}", texts(&rt));
        rt.update(|s, _| s.sync_still("synced", Some("me/repohub-notes".into())));
        rt.frame(0.1);
        assert!(has(&rt, "saved on this computer and synced through me/repohub-notes. Each branch"), "{:?}", texts(&rt));
    }

    #[test]
    fn settings_warns_about_a_public_repository_and_stop_syncing_leaves_everything() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _dir = crate::load_gen_tests::TempDataDir::new("sync-settings");
        let mut s = app();
        s.page = Page::Settings;
        s.settings_section = SettingsSection::Account;
        let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 3000.0), rune::Env::new(true, 1.0));
        rt.frame(0.0);
        assert!(rt.find("boards-sync").is_some());
        assert!(has(&rt, "Sync needs your GitHub account"), "sample data explains why");
        assert!(rt.find("notes-repo").is_none());
        // With an account: empty means off, with the link to create one.
        rt.update(|s, _| s.sync_still("setup", Some("me/repohub-notes".into())));
        rt.frame(0.1);
        assert!(rt.find("notes-repo").is_some() && rt.find("notes-repo-use").is_some() && rt.find("notes-repo-create").is_some());
        // A public repository: warned, and nothing is chosen until confirmed.
        rt.update(|s, _| s.sync.check = NotesRepoCheck::ConfirmPublic("me/repohub-notes".into()));
        rt.frame(0.2);
        assert!(has(&rt, "me/repohub-notes is public. Everyone could see your boards."));
        assert_eq!(rt.state.sync.repo, None);
        press(&mut rt, "notes-repo-public-ok");
        assert_eq!(rt.state.sync.repo.as_deref(), Some("me/repohub-notes"));
        assert_eq!(crate::sync_io::saved_choice(&rt.state.data.profile.login).as_deref(), Some("me/repohub-notes"));
        assert!(has(&rt, "Syncing through me/repohub-notes"));
        // Stop syncing forgets the choice; the boards stay.
        pollster::block_on(notes::save("octo/app", "main", "Plan", "{\"type\":\"excalidraw\",\"elements\":[]}".into())).unwrap();
        press(&mut rt, "stop-syncing");
        assert_eq!(rt.state.sync.repo, None);
        assert_eq!(rt.state.sync.status, BoardSync::Off);
        assert_eq!(crate::sync_io::saved_choice(&rt.state.data.profile.login), None);
        assert!(pollster::block_on(notes::load("octo/app", "main", "Plan")).unwrap().is_some(), "the board is still here");
        assert!(rt.state.toast.as_ref().is_some_and(|t| t.text.contains("They stay here and in me/repohub-notes")));
    }
}
