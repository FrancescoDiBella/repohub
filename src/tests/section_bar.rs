//! The repository's section bar (#65): the section on screen is always in view, and arrows say
//! there's more when the sections don't fit.

use crate::state::RepoTab;
use crate::{App, Page};

fn strip(rt: &rune::Runtime<App>) -> [f32; 4] {
    rt.elements().iter().find(|e| e.key.as_deref() == Some("tabs-strip")).expect("the strip").rect
}

fn settle(rt: &mut rune::Runtime<App>, t: &mut f64) {
    for _ in 0..8 {
        *t += 0.05;
        rt.frame(*t);
    }
}

fn in_view(rt: &rune::Runtime<App>, n: usize) -> bool {
    let [x, _, w, _] = strip(rt);
    rt.find(&format!("segment{n}")).is_some_and(|(cx, _)| cx > x && cx < x + w)
}

#[test]
fn the_section_on_screen_stays_in_view_and_arrows_say_there_is_more() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    for compact in [true, false] {
        let mut s = crate::init();
        s.data = crate::model::Data::sample();
        s.data_changed();
        s.pending_place = None;
        s.compact = compact;
        s.open(0);
        let size = if compact { (390.0, 844.0) } else { (1180.0, 900.0) };
        let mut rt = rune::Runtime::new(s, crate::view, size, rune::Env::new(true, 1.0));
        rt.reduced_motion = true;
        // The frame loop's poll scrolls the bar (`reveal_tab`), as the window's does.
        rt.poll = Some(crate::poll);
        let mut t = 0.0;
        rt.frame(t);
        settle(&mut rt, &mut t);
        assert!(rt.state.section_bar.overflows(), "eleven sections don't fit ({compact})");
        assert!(rt.find("tabs-before").is_some() && rt.find("tabs-after").is_some(), "an arrow at each end");
        assert!(rt.state.section_bar.hidden_after() && !rt.state.section_bar.hidden_before(), "more after the first ones, none before");
        assert!(in_view(&rt, 0));
        // A section past the right edge, opened from elsewhere (the palette, a plugin): scrolled to.
        let n = rt.state.tabs().iter().position(|t| *t == RepoTab::Terminal).unwrap();
        assert!(!in_view(&rt, n), "Terminal starts out of view ({compact})");
        rt.update(|s, _| s.set_tab(0, RepoTab::Terminal));
        settle(&mut rt, &mut t);
        assert!(in_view(&rt, n), "the section on screen comes into view ({compact})");
        assert!(rt.state.section_bar.hidden_before(), "now there are sections before it");
        // The left arrow goes back to the first ones.
        for _ in 0..4 {
            if in_view(&rt, 0) {
                break;
            }
            let (x, y) = rt.find("tabs-before").unwrap();
            rt.pointer_move(x, y);
            rt.pointer_down(x, y);
            rt.pointer_up(x, y);
            settle(&mut rt, &mut t);
        }
        assert!(in_view(&rt, 0), "the arrow scrolls back ({compact})");
        assert_eq!(rt.state.page, Page::Repo(rt.state.data.repos[0].full_name.clone()));
    }
}
