//! Filesystem-watch primitives shared by the two type watchers.
//!
//! `suprnova serve` runs one watcher (`commands::serve::start_type_watcher`)
//! and `suprnova generate-types --watch` runs another
//! (`commands::generate_types::start_watcher`). They watch the same trees for
//! the same reasons, and both of them shipped the same two defects: they
//! counted a `notify` event by its path alone, and they had no way to tell a
//! burst of saves from a single one. Keeping one implementation here is what
//! stops the next fix from landing in only one of them.

use std::time::{Duration, Instant};

/// How long a burst of saves must be quiet before a regeneration runs.
///
/// One value for both watchers on purpose: `serve` and
/// `generate-types --watch` are the same job started two ways, and a user
/// who learns the feel of one should not have to relearn it for the other.
pub(crate) const REGEN_QUIET: Duration = Duration::from_millis(500);

/// What one filesystem event asks the type watcher to regenerate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WatchTrigger {
    /// Regenerate `inertia-props.ts`: a `.rs` file changed.
    pub(crate) rust: bool,
    /// Regenerate `lang-keys.ts`: a `.ftl` catalog changed.
    pub(crate) ftl: bool,
}

impl WatchTrigger {
    /// Nothing to regenerate.
    pub(crate) const NONE: Self = Self {
        rust: false,
        ftl: false,
    };
}

/// Classify one `notify` event into what it should regenerate.
///
/// The event *kind* gate is what keeps this watcher from feeding itself.
/// `notify`'s inotify backend registers `WatchMask::OPEN` on every watched
/// directory, so the kernel reports a plain read of a file as
/// `Access(Open(..))` followed by `Access(Close(Read))`. Regeneration
/// reads every `.rs` file under `src/` - the exact tree this watcher is
/// watching - so each run emits a burst of those `Access` events on the
/// watcher's own channel. Counting them as changes re-arms the
/// trailing-edge debounce, which fires another regeneration 500ms later,
/// which reads the tree again: a project nobody has touched rebuilds its
/// types (and, through cargo-watch, its backend) forever.
///
/// Only kinds that mean the bytes on disk are different now count:
/// `Create`, `Modify`, `Remove`, and `Any` (the backends' "we do not know
/// what this was" catch-all, which must stay conservative). `Access` and
/// `Other` never do.
pub(crate) fn watch_trigger(event: &notify::Event) -> WatchTrigger {
    let is_change = matches!(
        event.kind,
        notify::EventKind::Create(_)
            | notify::EventKind::Modify(_)
            | notify::EventKind::Remove(_)
            | notify::EventKind::Any
    );
    if !is_change {
        return WatchTrigger::NONE;
    }

    let has_extension = |ext: &str| {
        event
            .paths
            .iter()
            .any(|p| p.extension().map(|e| e == ext).unwrap_or(false))
    };

    WatchTrigger {
        rust: has_extension("rs"),
        ftl: has_extension("ftl"),
    }
}

/// Trailing-edge debounce: fire once the burst has gone quiet.
///
/// The watcher used to debounce on the *leading* edge -
/// `if is_rust_change && last_regen.elapsed() > debounce_duration` - which
/// regenerates on the first event of a burst and then silently drops every
/// event for the next 500ms with no trailing run.
///
/// That loses work rather than merely delaying it, and it loses the work
/// most likely to matter. A burst is not a rare event: `cargo fmt`,
/// format-on-save across several files, a branch switch, and any editor
/// that writes a temp file and renames it all produce one. The regenerate
/// fires on the *first* file, before the rest are written, so the types on
/// disk reflect a partial edit - and nothing regenerates them until some
/// unrelated future save happens to land outside a quiet window. The
/// developer sees stale types and no error.
///
/// Firing on the trailing edge inverts that: the burst is coalesced into
/// exactly one regeneration, and it runs after the last write.
pub(crate) struct Debounce {
    /// How long the burst must be quiet before firing.
    quiet: Duration,
    /// When the most recent event arrived, if one is waiting to fire.
    pending_since: Option<Instant>,
}

impl Debounce {
    pub(crate) fn new(quiet: Duration) -> Self {
        Self {
            quiet,
            pending_since: None,
        }
    }

    /// Record an event. Each one restarts the quiet period, so a steady
    /// stream of saves coalesces into a single run after the last.
    pub(crate) fn on_event(&mut self, now: Instant) {
        self.pending_since = Some(now);
    }

    /// Whether the pending burst has gone quiet long enough to fire.
    ///
    /// Consumes the pending flag, so one burst produces exactly one run.
    pub(crate) fn should_fire(&mut self, now: Instant) -> bool {
        match self.pending_since {
            Some(last) if now.duration_since(last) >= self.quiet => {
                self.pending_since = None;
                true
            }
            _ => false,
        }
    }
}

/// The two independent trailing-edge debounces a type watcher needs, driven
/// straight from `notify` events.
///
/// This exists so the per-event decision both watchers make is one testable
/// unit rather than two hand-written loops. `observe` classifies and arms;
/// `due` reports what has gone quiet long enough to run. The two artifacts
/// are debounced separately because a `.rs` save should not reparse every
/// `.ftl` catalog, and vice versa.
pub(crate) struct RegenerationSchedule {
    rust: Debounce,
    ftl: Debounce,
}

impl RegenerationSchedule {
    /// A schedule whose bursts must be quiet for `quiet` before firing.
    pub(crate) fn new(quiet: Duration) -> Self {
        Self {
            rust: Debounce::new(quiet),
            ftl: Debounce::new(quiet),
        }
    }

    /// Record one filesystem event, arming whichever regenerations it asks
    /// for. An event that is not a real change arms nothing - see
    /// [`watch_trigger`].
    pub(crate) fn observe(&mut self, event: &notify::Event, now: Instant) {
        let trigger = watch_trigger(event);
        if trigger.rust {
            self.rust.on_event(now);
        }
        if trigger.ftl {
            self.ftl.on_event(now);
        }
    }

    /// What should run now. Each armed burst yields exactly once.
    pub(crate) fn due(&mut self, now: Instant) -> WatchTrigger {
        WatchTrigger {
            rust: self.rust.should_fire(now),
            ftl: self.ftl.should_fire(now),
        }
    }
}

#[cfg(test)]
mod regeneration_schedule_tests {
    //! The per-event decision both watchers make, driven with explicit
    //! `Instant`s so these take no wall-clock time.

    use super::*;
    use notify::EventKind;
    use notify::event::{AccessKind, AccessMode, CreateKind, DataChange, ModifyKind};
    use std::path::PathBuf;

    fn event_on(kind: EventKind, path: &str) -> notify::Event {
        notify::Event::new(kind).add_path(PathBuf::from(path))
    }

    fn wrote(path: &str) -> notify::Event {
        event_on(
            EventKind::Modify(ModifyKind::Data(DataChange::Content)),
            path,
        )
    }

    #[test]
    fn an_access_event_arms_nothing() {
        // `generate-types --watch` regenerated on any event whose path
        // ended in `.rs`, and the generator reads every `.rs` file under
        // the tree it watches. On Linux that is a closed loop.
        let mut schedule = RegenerationSchedule::new(REGEN_QUIET);
        let t0 = Instant::now();

        for kind in [
            EventKind::Access(AccessKind::Open(AccessMode::Any)),
            EventKind::Access(AccessKind::Close(AccessMode::Read)),
            EventKind::Access(AccessKind::Read),
        ] {
            schedule.observe(&event_on(kind, "src/a.rs"), t0);
            schedule.observe(&event_on(kind, "lang/en/x.ftl"), t0);
        }

        assert_eq!(
            schedule.due(t0 + Duration::from_secs(60)),
            WatchTrigger::NONE,
            "reads must never schedule a regeneration, however long we wait"
        );
    }

    #[test]
    fn a_write_fires_once_the_burst_goes_quiet() {
        let mut schedule = RegenerationSchedule::new(REGEN_QUIET);
        let t0 = Instant::now();
        schedule.observe(&wrote("src/a.rs"), t0);

        assert_eq!(
            schedule.due(t0 + Duration::from_millis(100)),
            WatchTrigger::NONE,
            "still inside the quiet period"
        );
        assert_eq!(
            schedule.due(t0 + REGEN_QUIET),
            WatchTrigger {
                rust: true,
                ftl: false
            }
        );
        assert_eq!(
            schedule.due(t0 + REGEN_QUIET + REGEN_QUIET),
            WatchTrigger::NONE,
            "one burst produces exactly one run"
        );
    }

    #[test]
    fn a_burst_of_saves_collapses_into_one_run() {
        // `cargo fmt`, format-on-save across a few files, a branch switch.
        let mut schedule = RegenerationSchedule::new(REGEN_QUIET);
        let t0 = Instant::now();
        for step in 0..5 {
            schedule.observe(&wrote("src/a.rs"), t0 + Duration::from_millis(step * 100));
            assert_eq!(
                schedule.due(t0 + Duration::from_millis(step * 100)),
                WatchTrigger::NONE,
                "must not fire mid-burst, on a half-written tree"
            );
        }
        let last = t0 + Duration::from_millis(400);
        assert!(
            schedule.due(last + REGEN_QUIET).rust,
            "the run happens after the last write, not the first"
        );
    }

    #[test]
    fn the_two_artifacts_are_scheduled_independently() {
        let mut schedule = RegenerationSchedule::new(REGEN_QUIET);
        let t0 = Instant::now();
        schedule.observe(&wrote("lang/en/app.ftl"), t0);
        schedule.observe(
            &event_on(EventKind::Create(CreateKind::File), "src/b.rs"),
            t0,
        );

        assert_eq!(
            schedule.due(t0 + REGEN_QUIET),
            WatchTrigger {
                rust: true,
                ftl: true
            }
        );
    }

    #[test]
    fn a_file_that_is_neither_rust_nor_fluent_arms_nothing() {
        let mut schedule = RegenerationSchedule::new(REGEN_QUIET);
        let t0 = Instant::now();
        schedule.observe(&wrote("src/notes.md"), t0);
        assert_eq!(schedule.due(t0 + REGEN_QUIET), WatchTrigger::NONE);
    }
}

#[cfg(test)]
mod watch_trigger_tests {
    use super::*;
    use notify::event::{AccessKind, AccessMode, CreateKind, DataChange, ModifyKind, RemoveKind};
    use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
    use std::path::PathBuf;
    use std::sync::mpsc::channel;

    fn event_on(kind: EventKind, path: &str) -> notify::Event {
        notify::Event::new(kind).add_path(PathBuf::from(path))
    }

    /// The three kinds that mean "the bytes on disk are different now".
    fn real_changes() -> [EventKind; 3] {
        [
            EventKind::Modify(ModifyKind::Data(DataChange::Content)),
            EventKind::Create(CreateKind::File),
            EventKind::Remove(RemoveKind::File),
        ]
    }

    #[test]
    fn reading_a_rust_file_is_not_a_change() {
        // This is the loop: the generator opens and reads every `.rs`
        // file under the tree this watcher is watching, so its own reads
        // must not schedule the next regeneration.
        for kind in [
            EventKind::Access(AccessKind::Open(AccessMode::Any)),
            EventKind::Access(AccessKind::Close(AccessMode::Read)),
        ] {
            assert_eq!(
                watch_trigger(&event_on(kind, "src/a.rs")),
                WatchTrigger::NONE,
                "{kind:?} on a .rs file must not trigger a regeneration"
            );
        }
    }

    #[test]
    fn writing_creating_or_removing_a_rust_file_triggers_types() {
        for kind in real_changes() {
            let trigger = watch_trigger(&event_on(kind, "src/a.rs"));
            assert!(trigger.rust, "{kind:?} on a .rs file must regenerate types");
            assert!(!trigger.ftl, "{kind:?} on a .rs file is not a lang change");
        }
    }

    #[test]
    fn writing_creating_or_removing_a_catalog_triggers_lang_keys() {
        for kind in real_changes() {
            let trigger = watch_trigger(&event_on(kind, "lang/en/x.ftl"));
            assert!(
                trigger.ftl,
                "{kind:?} on a .ftl file must regenerate lang keys"
            );
            assert!(
                !trigger.rust,
                "{kind:?} on a .ftl file is not a Rust change"
            );
        }
    }

    #[test]
    fn a_file_that_is_neither_rust_nor_fluent_triggers_nothing() {
        assert_eq!(
            watch_trigger(&event_on(
                EventKind::Modify(ModifyKind::Data(DataChange::Content)),
                "src/notes.md"
            )),
            WatchTrigger::NONE
        );
    }

    /// End-to-end against a real inotify watcher, because the bug lived in
    /// the gap between what `notify` emits and what the classifier looked
    /// at: nothing that only builds `Event` values by hand can prove the
    /// kernel does not hand us an `Access` event for a plain read.
    ///
    /// Linux-only: the `OPEN`/`CLOSE` watch mask is an inotify detail, and
    /// the other backends do not report reads at all.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_real_read_of_a_watched_file_produces_no_trigger_but_a_write_does() {
        use std::sync::mpsc::RecvTimeoutError;

        let dir = tempfile::tempdir().expect("tempdir");
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).expect("create src");
        let file = src.join("a.rs");
        std::fs::write(&file, "pub struct A;\n").expect("seed a.rs");

        let (tx, rx) = channel();
        let watcher_result = RecommendedWatcher::new(
            move |res| {
                if let Ok(event) = res {
                    let _ = tx.send(event);
                }
            },
            Config::default().with_poll_interval(Duration::from_secs(2)),
        );
        let mut watcher = match watcher_result {
            Ok(w) => w,
            Err(e) => {
                println!("skipping: no filesystem watcher available ({e})");
                return;
            }
        };
        if let Err(e) = watcher.watch(&src, RecursiveMode::Recursive) {
            println!("skipping: cannot watch a temp dir ({e})");
            return;
        }

        // Drain whatever registering the watch produced.
        while rx.recv_timeout(Duration::from_millis(200)).is_ok() {}

        // A read is exactly what the generator does to every file under
        // the watched tree on each run.
        let _ = std::fs::read_to_string(&file).expect("read a.rs");
        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(event) => assert_eq!(
                    watch_trigger(&event),
                    WatchTrigger::NONE,
                    "reading a watched file must not schedule a regeneration, got {:?}",
                    event.kind
                ),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }

        // A write must still get through, or the fix would have traded a
        // loop for a dead watcher.
        std::fs::write(&file, "pub struct A;\npub struct B;\n").expect("rewrite a.rs");
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut saw_change = false;
        while !saw_change && Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(event) => saw_change = watch_trigger(&event).rust,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        assert!(saw_change, "a write to a watched .rs file must trigger");
    }
}

#[cfg(test)]
mod debounce_tests {
    //! P2-13. The watcher debounced on the leading edge, which does not
    //! delay work - it discards it. These drive `Debounce` with explicit
    //! `Instant`s, so they are deterministic and take no wall-clock time.

    use super::Debounce;
    use std::time::{Duration, Instant};

    const QUIET: Duration = Duration::from_millis(500);

    #[test]
    fn nothing_fires_without_an_event() {
        let mut d = Debounce::new(QUIET);
        let t0 = Instant::now();

        assert!(!d.should_fire(t0));
        assert!(
            !d.should_fire(t0 + Duration::from_secs(60)),
            "an idle watcher must never regenerate - the quiet period is \
             measured from an event, not from process start"
        );
    }

    #[test]
    fn a_single_event_fires_once_after_the_quiet_period() {
        let mut d = Debounce::new(QUIET);
        let t0 = Instant::now();
        d.on_event(t0);

        assert!(
            !d.should_fire(t0 + Duration::from_millis(499)),
            "must not fire before the quiet period elapses"
        );
        assert!(d.should_fire(t0 + Duration::from_millis(500)));
        assert!(
            !d.should_fire(t0 + Duration::from_secs(10)),
            "one event must produce exactly one run, not a repeating timer"
        );
    }

    /// The regression, stated directly. A burst of saves must produce one
    /// regeneration, and it must happen *after the last one* - that is the
    /// save whose types were previously lost.
    #[test]
    fn a_burst_fires_once_and_only_after_its_final_event() {
        let mut d = Debounce::new(QUIET);
        let t0 = Instant::now();

        // A burst: five saves 100ms apart. `cargo fmt` across a few files
        // looks exactly like this.
        for i in 0..5 {
            let at = t0 + Duration::from_millis(i * 100);
            d.on_event(at);
            assert!(
                !d.should_fire(at),
                "firing at event {i} would regenerate from a partially \
                 written burst - the leading-edge bug"
            );
        }

        let last_event = t0 + Duration::from_millis(400);
        assert!(
            !d.should_fire(last_event + Duration::from_millis(499)),
            "the quiet period restarts on every event, so it is measured \
             from the LAST save, not the first"
        );
        assert!(
            d.should_fire(last_event + Duration::from_millis(500)),
            "the burst must regenerate once it goes quiet; under the old \
             leading-edge debounce the final four saves were dropped and \
             nothing regenerated them"
        );
        assert!(
            !d.should_fire(last_event + Duration::from_secs(10)),
            "and exactly once"
        );
    }

    /// A save arriving during the quiet period extends it rather than
    /// being swallowed. This is the case the old code got wrong: it
    /// dropped these events entirely.
    #[test]
    fn an_event_during_the_quiet_period_is_not_lost() {
        let mut d = Debounce::new(QUIET);
        let t0 = Instant::now();
        d.on_event(t0);

        // 300ms in - inside the old 500ms window, where this event used
        // to be discarded outright.
        let second = t0 + Duration::from_millis(300);
        d.on_event(second);

        assert!(
            !d.should_fire(t0 + Duration::from_millis(500)),
            "the window must have been extended by the second event"
        );
        assert!(
            d.should_fire(second + QUIET),
            "and the fire must come after the SECOND event, so its changes \
             are included"
        );
    }

    /// Separate bursts each get their own run.
    #[test]
    fn a_later_burst_fires_again() {
        let mut d = Debounce::new(QUIET);
        let t0 = Instant::now();

        d.on_event(t0);
        assert!(d.should_fire(t0 + QUIET));

        let later = t0 + Duration::from_secs(30);
        d.on_event(later);
        assert!(!d.should_fire(later));
        assert!(d.should_fire(later + QUIET));
    }
}
