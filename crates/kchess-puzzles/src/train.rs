//! The Train tab's session: what it asks the core for, what it does with the answers, and the
//! choices of what to train.

use super::*;

impl PuzzlesPage {
    pub(super) fn mode(cx: &App) -> String {
        crate::prefs::get::<String>(cx, MODE, "practice".into())
    }

    /// The account picker lists the connected accounts, with the one rated puzzles go to.
    pub(super) fn sync_accounts(&self, window: &mut Window, cx: &mut Context<Self>) {
        let choices: Vec<Choice> = self
            .accounts(cx)
            .into_iter()
            .map(|name| Choice::new(name.clone(), format!("@{name}")))
            .collect();
        let account = SharedString::from(self.account(cx));
        self.account_select.update(cx, |state, cx| {
            state.set_items(choices, window, cx);
            state.set_selected_value(&account, window, cx);
        });
    }

    /// The connected accounts.
    pub(super) fn accounts(&self, cx: &App) -> Vec<String> {
        crate::accounts::handle(cx).read(cx).connected()
    }

    /// The connected account rated puzzles go to.
    pub(super) fn account(&self, cx: &App) -> String {
        let preferred = crate::prefs::get::<String>(cx, ACCOUNT, String::new());
        let connected = self.accounts(cx);
        let accounts: Vec<ListedAccount> = connected
            .iter()
            .map(|username| ListedAccount {
                username: Some(username),
                connected: true,
            })
            .collect();
        connected_account_index(&accounts, Some(&preferred))
            .and_then(|index| connected.get(index).cloned())
            .unwrap_or_default()
    }

    fn selection(&self, cx: &App) -> Selection {
        Selection {
            account: self.account(cx),
            angle: crate::prefs::get::<String>(cx, ANGLE, "mix".into()),
            difficulty: crate::prefs::get::<String>(cx, DIFFICULTY, "normal".into()),
            color: crate::prefs::get::<String>(cx, COLOR, "random".into()),
            mode: Self::mode(cx),
        }
    }

    pub(super) fn load_next(&mut self, cx: &mut Context<Self>) {
        let step = self
            .session
            .load_begin(&self.selection(cx), true, &local_day());
        self.follow(step, Settle::Load, cx);
        self.sync_board(cx);
    }

    /// Run the steps' host calls in order until one asks for nothing more.
    pub(super) fn follow(&mut self, step: Step, settle: Settle, cx: &mut Context<Self>) {
        if let Some(record) = step.record {
            self.record(record, cx);
        }
        let Some(call) = step.call else {
            return;
        };
        let (method, request) = puzzle_request(cx.global::<AppCore>(), &call.request);
        cx.spawn(async move |this, cx| {
            let reply = request.await.map_err(|cause| {
                crate::log::write(
                    Level::Warn,
                    "puzzles",
                    &format!("{method} failed: {}", cause.message),
                );
                Failure {
                    message: cause.message,
                    kind: cause.kind,
                }
            });
            let _ = this.update(cx, |this, cx| {
                let selection = this.selection(cx);
                let next = settle.apply(&mut this.session, call.resume, reply, &selection);
                this.follow(next, settle, cx);
                this.sync_board(cx);
            });
        })
        .detach();
    }
}

/// A core call's answer, on its way.
type Sent = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<Answer, kchess_contracts::CoreError>>>,
>;

/// The session's value as the contract's type of it, read by the contract's own check as the
/// core reads what is sent to it (a whole rating written `1500.0` is the integer 1500).
pub(super) fn as_contract<T: kchess_contracts::check::Validate>(
    value: &impl serde::Serialize,
) -> Result<T, kchess_contracts::CoreError> {
    let value = serde_json::to_value(value)
        .map_err(|cause| kchess_contracts::CoreError::new(cause.to_string()))?;
    T::validate(kchess_domain::misc::value::J::from(&value))
        .map_err(kchess_contracts::CoreError::new)
}

/// A call sent, its answer as the session reads it.
fn answer<C: kchess_contracts::calls::CoreCall + Send + 'static>(
    core: &AppCore,
    call: Result<C, kchess_contracts::CoreError>,
    read: fn(C::Answer) -> Answer,
) -> Sent
where
    C::Answer: Send,
{
    let sent = call.map(|call| core.request(call));
    Box::pin(async move { sent?.await.map(read) })
}

/// Lichess's answer, or that the account must be reconnected.
fn lichess<T>(reply: kchess_contracts::types::Reply<T>, read: impl FnOnce(T) -> Answer) -> Answer {
    match reply {
        kchess_contracts::types::Reply::Done(answer) => read(answer),
        kchess_contracts::types::Reply::NeedsReconnect => Answer::NeedsReconnect,
    }
}

/// A puzzle step's core call, sent, and its method's name.
fn puzzle_request(core: &AppCore, request: &Request) -> (&'static str, Sent) {
    use kchess_contracts::calls::CoreCall;
    match request {
        Request::LocalPuzzles(query) => (
            calls::LocalPuzzles::METHOD,
            answer(
                core,
                as_contract(query).map(|query| calls::LocalPuzzles { query }),
                Answer::Local,
            ),
        ),
        Request::PuzzleNext(draw) => (
            calls::PuzzleNext::METHOD,
            answer(
                core,
                as_contract(draw).map(|request| calls::PuzzleNext { request }),
                |reply| {
                    lichess(reply, |draw| Answer::Drawn {
                        puzzle: draw.puzzle,
                        rating: draw.glicko.and_then(|glicko| glicko.rating),
                    })
                },
            ),
        ),
        Request::PuzzleSolve(solve) => (
            calls::PuzzleSolve::METHOD,
            answer(
                core,
                as_contract(solve).map(|request| calls::PuzzleSolve { request }),
                |reply| {
                    lichess(reply, |solved| Answer::Solved {
                        rating_diff: solved.rating_diff.flatten().map(|diff| diff as f64),
                    })
                },
            ),
        ),
    }
}

impl PuzzlesPage {
    /// Keep an attempt in the local log; a failed write never interrupts training.
    fn record(&mut self, attempt: Value, cx: &mut Context<Self>) {
        let save = as_contract(&attempt).map(|attempt| {
            cx.global::<AppCore>()
                .request(calls::RecordPuzzleAttempt { attempt })
        });
        cx.spawn(async move |this, cx| match async { save?.await }.await {
            Ok(progress) => {
                let _ = this.update(cx, |this, cx| {
                    this.progress = progress;
                    cx.notify();
                });
            }
            Err(cause) => crate::log::write(
                Level::Warn,
                "puzzles",
                &format!("recording the attempt failed: {}", cause.message),
            ),
        })
        .detach();
    }

    pub(super) fn report(&mut self, win: bool, detail: Detail, cx: &mut Context<Self>) {
        let step = self
            .session
            .report_begin(win, Some(detail), &self.selection(cx));
        self.follow(step, Settle::Report, cx);
    }

    /* ── The board ────────────────────────────────────────────────────── */

    /// A new puzzle (a new key) starts the board afresh.
    fn sync_board(&mut self, cx: &mut Context<Self>) {
        let ready = self.session.phase() == Phase::Ready;
        let key = self.session.puzzle_key();
        let shown = self.puzzle.read(cx).puzzle().is_some();
        match self.session.current().cloned() {
            Some(puzzle) if ready && (!shown || key != self.shown_key) => {
                self.shown_key = key;
                let outcome = self.session.state().attempt_outcome;
                self.puzzle
                    .update(cx, |board, cx| board.start(puzzle, outcome, cx));
            }
            _ if !ready => self.puzzle.update(cx, |board, cx| board.clear(cx)),
            _ => {}
        }
        cx.notify();
    }

    /// The puzzle is over: count and log it, and go on to the next one after a solve when asked.
    pub(super) fn done(&mut self, cx: &mut Context<Self>) {
        let step = self.session.end(&local_day());
        self.follow(step, Settle::Load, cx);
        let solved = self.puzzle.read(cx).status() == PuzzleStatus::Solved;
        let auto_next = crate::settings::frontend(cx).puzzle_auto_next;
        if solved && auto_next {
            let key = self.session.puzzle_key();
            // An earlier puzzle's countdown has nothing left to do (it was replaced): dropped,
            // not kept with every puzzle solved since the page opened.
            self.timers.clear();
            self.timers.push(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(AUTO_NEXT_DELAY).await;
                let _ = this.update(cx, |this, cx| {
                    // Moving on by hand, or changing what to train, already replaced the puzzle.
                    if this.session.puzzle_key() == key && this.session.phase() == Phase::Ready {
                        this.load_next(cx);
                    }
                });
            }));
        }
    }

    /// The Lichess stats view, made when first needed (the Stats tab, and Rush's results).
    pub(super) fn stats_view(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<stats::PuzzleStats> {
        if let Some(stats) = &self.stats {
            return stats.clone();
        }
        let themes = self.theme_list.clone();
        let stats = cx.new(|cx| stats::PuzzleStats::new(themes, window, cx));
        self._subscriptions.push(cx.subscribe_in(
            &stats,
            window,
            |this, view, event: &stats::StatsEvent, window, cx| match event {
                stats::StatsEvent::Train(key) => {
                    let key = key.clone();
                    this.train_theme(&key, window, cx);
                }
                stats::StatsEvent::Reconnect => {
                    crate::accounts::handle(cx)
                        .update(cx, |accounts, cx| accounts.connect(window, cx));
                }
                stats::StatsEvent::Account(name) => {
                    crate::prefs::set(cx, ACCOUNT, name.clone());
                    this.sync_accounts(window, cx);
                    let (account, accounts) = (this.account(cx), this.accounts(cx));
                    view.update(cx, |stats, cx| stats.shown(account, accounts, window, cx));
                    this.load_next(cx);
                }
            },
        ));
        self.stats = Some(stats.clone());
        stats
    }

    /// `retry`: play an earlier puzzle again on the Train tab (unrated, not counted).
    pub(crate) fn retry(&mut self, puzzle: Puzzle, cx: &mut Context<Self>) {
        Tab::Train.store(cx);
        let step = self
            .session
            .retry(puzzle, &self.selection(cx), &local_day());
        self.follow(step, Settle::Load, cx);
        self.sync_board(cx);
        cx.notify();
    }

    /// Train any theme from the downloaded puzzles.
    pub(super) fn use_downloaded(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        Tab::Train.store(cx);
        crate::prefs::set(cx, MODE, "offline");
        self.choose_angle("mix", window, cx);
    }

    pub(super) fn choose_mode(&mut self, mode: &'static str, cx: &mut Context<Self>) {
        crate::prefs::set(cx, MODE, mode);
        self.load_next(cx);
    }

    /// Train another theme, as choosing it in the select does.
    pub(super) fn choose_angle(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        crate::prefs::set(cx, ANGLE, key);
        let key = SharedString::from(key.to_string());
        self.theme_select.update(cx, |state, cx| {
            state.set_selected_value(&key, window, cx);
            if state.selected_value() != Some(&key) {
                state.set_selected_index(Some(IndexPath::default()), window, cx);
            }
        });
        self.load_next(cx);
    }

    /// Train a theme from another tab or a finished puzzle's themes: back on Train, with it chosen.
    pub(crate) fn train_theme(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        Tab::Train.store(cx);
        self.choose_angle(key, window, cx);
    }

    pub(super) fn choose_difficulty(
        &mut self,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        crate::prefs::set(cx, DIFFICULTY, key);
        let index = DIFFICULTIES
            .iter()
            .position(|(value, _)| *value == key)
            .unwrap_or(2);
        self.difficulty_select.update(cx, |state, cx| {
            state.set_selected_index(Some(IndexPath::new(index)), window, cx)
        });
        self.load_next(cx);
    }

    /// Remember the opening an opening page asked for puzzles of, as the theme to
    /// train; a page made after this starts with it.
    pub fn choose_opening(key: &str, name: &str, cx: &mut App) {
        crate::prefs::set(
            cx,
            OPENING,
            OpeningAngle {
                key: key.to_string(),
                name: name.to_string(),
            },
        );
        crate::prefs::set(cx, ANGLE, key);
        Tab::Train.store(cx);
    }

    /// The opening chosen with [`Self::choose_opening`], on a page made before it.
    pub fn opening_chosen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.theme_list = theme_items(cx);
        let choices = theme_choices(cx);
        self.theme_select
            .update(cx, |state, cx| state.set_items(choices, window, cx));
        let angle = crate::prefs::get::<String>(cx, ANGLE, "mix".into());
        self.choose_angle(&angle, window, cx);
    }

    pub(super) fn reset_session(&mut self, cx: &mut Context<Self>) {
        let step = self.session.reset_session();
        self.follow(step, Settle::Load, cx);
        cx.notify();
    }
}

/// Train a theme on the Puzzles page, after the click that asked for it.
pub(super) fn train_theme(key: String, window: &mut Window, cx: &mut App) {
    window.defer(cx, move |window, cx| {
        if let Some(page) = kchess_nav::shell::view::<PuzzlesPage>(cx) {
            page.update(cx, |page, cx| page.train_theme(&key, window, cx));
        }
    });
}

/// Which reply a host call settles: a load, or Lichess's answer to a result.
#[derive(Clone, Copy)]
pub(super) enum Settle {
    Load,
    Report,
}

impl Settle {
    fn apply(
        self,
        session: &mut PuzzleSession,
        resume: Resume,
        reply: Reply,
        selection: &Selection,
    ) -> Step {
        match self {
            Settle::Load => session.load_settled(resume, reply, selection, &local_day()),
            Settle::Report => session.report_settled(resume, reply, selection),
        }
    }
}
