//! Keeping the viewer current: the sessions list, the shown session's feed, commands from
//! `ichnos open` and the replay tick.

use std::sync::{Arc, PoisonError};

use gpui_kit::{Context, Window};
use ichnos_core::catalog::SessionEntry;
use ichnos_core::instance::{Command, Listener};
use ichnos_core::store::SessionId;
use ichnos_model::model::moments;

use super::{COPIED_FOR, FRESH, POLL_INTERVAL, POLLS_PER_TICK, SESSIONS_POLL, Viewer};
use crate::live::Connection;

impl Viewer {
    pub(super) fn poll_sessions(&mut self, cx: &mut Context<Self>) {
        let catalog = Arc::clone(&self.catalog);
        cx.spawn(async move |this, cx| {
            loop {
                let catalog = Arc::clone(&catalog);
                let listed = cx
                    .background_executor()
                    .spawn(async move {
                        catalog
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .list()
                    })
                    .await;
                let updated = this.update(cx, |viewer, cx| {
                    if let Ok(sessions) = listed {
                        viewer.on_sessions(sessions, cx);
                    }
                });
                if updated.is_err() {
                    break;
                }
                cx.background_executor().timer(SESSIONS_POLL).await;
            }
        })
        .detach();
    }

    fn on_sessions(&mut self, sessions: Vec<SessionEntry>, cx: &mut Context<Self>) {
        self.sessions = sessions;
        let newest = self.sessions.first().map(|s| s.id().clone());
        if let Some(newest) = newest
            && (self.follow_newest || self.session.is_none())
        {
            self.show_session(newest, cx);
        }
        if let Some(switcher) = &self.switcher {
            let sessions = self.sessions.clone();
            switcher.update(cx, |switcher, cx| switcher.set_sessions(sessions, cx));
        }
        cx.notify();
    }

    pub(super) fn poll_feed(window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL_INTERVAL).await;
                let Ok(feed) = this.update(cx, |viewer, _| {
                    let live = viewer.live.as_mut()?;
                    live.reopen(&viewer.sessions_dir);
                    Some((live.id().clone(), live.feed()?))
                }) else {
                    break;
                };
                let polled = match feed {
                    Some((id, feed)) => {
                        let polled = cx
                            .background_executor()
                            .spawn(async move { crate::live::poll(&feed) })
                            .await;
                        Some((id, polled))
                    }
                    None => None,
                };
                if this
                    .update_in(cx, |viewer, window, cx| {
                        viewer.take_commands(window, cx);
                        viewer.on_tick(polled, cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    /// Act on the requests `ichnos open` and `ichnos stop` sent since the last poll.
    fn take_commands(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        while let Some(command) = self.listener.as_ref().and_then(Listener::try_recv) {
            match command {
                Command::Show { session, view } => {
                    if session.is_some() || !self.follow_newest {
                        self.open_session(session, view, cx);
                    } else if let Some(view) = view {
                        self.set_view(view, cx);
                    }
                    window.activate_window();
                }
                Command::Ping => {}
                Command::Quit => cx.quit(),
            }
        }
    }

    /// One poll's result, plus the clock: playback steps and running durations.
    fn on_tick(
        &mut self,
        polled: Option<(
            SessionId,
            Result<Vec<ichnos_model::feed::Item>, ichnos_core::store::StoreError>,
        )>,
        cx: &mut Context<Self>,
    ) {
        let mut dirty = false;
        if let (Some((id, polled)), Some(live)) = (polled, self.live.as_mut())
            && live.id() == &id
        {
            let was = live.connection();
            dirty |= live.apply(polled);
            dirty |= was != live.connection();
        }
        self.polls += 1;
        if self.state.playing() {
            let data = self.data();
            let all = moments(data.events(), data.steps());
            self.state.step(&all, true);
            dirty = true;
        }
        let ticking = self.state.replay_at().is_none()
            && self.connection() == Some(Connection::Live)
            && self
                .model
                .turns()
                .iter()
                .any(|&turn| self.model.node(turn).running());
        if ticking && self.polls.is_multiple_of(POLLS_PER_TICK) {
            dirty = true;
        }
        if self
            .copied
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() >= COPIED_FOR)
        {
            self.copied = None;
            dirty = true;
        }
        let fresh = self
            .seen
            .values()
            .flatten()
            .any(|at| at.elapsed() < FRESH + POLL_INTERVAL);
        if dirty || fresh {
            self.refresh();
            cx.notify();
        }
    }
}
