//! `Space a`: run Claude Code on this review (the dv-review skill), without leaving dv. Inside
//! zellij it gets a floating pane (an interactive session to watch and steer); elsewhere it runs
//! in the background. Either way its review shows up as it writes the file.
use super::*;
use crate::claude::{answer_prompt, claude_command, claude_dirs, data_env, fix_checks_prompt, path_for_claude, review_prompt, tools_for, update_prompt, with_instructions};

#[derive(Clone, Copy)]
pub(super) enum Ask {
    Review,
    Answer,
    /// Make the changes the review's open comments ask for (Claude's and yours), and/or what
    /// your instructions say.
    Fix,
    /// Propose a new description for the PR (in the review file), for you to approve.
    Describe,
    /// On someone else's PR: make the changes (as `Fix` does on yours), here, for you to push
    /// to their branch, rather than suggest them.
    Edit,
    /// Your PR's CI fails: find out why from the failed runs' logs, and fix it here.
    FixChecks,
    /// Draft a comment closing the PR (in the review file), for you to close it with.
    Close,
    /// An issue: triage it (again), into its file.
    Triage,
}

impl Ask {
    /// Its `Work` (its model: `dv.model.<key>`), as dv list's same tasks have (`Task::work`).
    pub(super) fn work(self) -> crate::claude::Work {
        use crate::claude::Work;
        match self {
            Ask::Review => Work::Review,
            Ask::Answer => Work::Answer,
            Ask::Fix | Ask::Edit => Work::Change,
            Ask::Describe => Work::Describe,
            Ask::FixChecks => Work::FixCi,
            Ask::Close => Work::Close,
            Ask::Triage => Work::Triage,
        }
    }
}

/// A background `claude -p` run.
pub(super) struct ClaudeJob {
    pub(super) child: std::process::Child,
    /// Its result (JSON); errors go next to it (`.err`).
    pub(super) log: PathBuf,
    /// What it's doing, for the status bar.
    pub(super) label: &'static str,
    /// It ran on a fork of the PR's session (the session itself is open in a pane), so its
    /// session id isn't the one to keep.
    pub(super) forked: bool,
    pub(super) started: std::time::Instant,
    /// Its hold on the PR (or issue): one run on it at a time, from any dv; let go with it.
    pub(super) _claim: Option<crate::claude::Claimed>,
    /// A triage's: the issue (as of when, with the PRs linked as fixing it); its answer kept in
    /// the issue's file when it ends (`kept`), however the view goes.
    pub(super) triage: Option<IssueRec>,
}

impl ClaudeJob {
    pub(super) fn new(child: std::process::Child, log: PathBuf, label: &'static str, forked: bool) -> ClaudeJob {
        ClaudeJob { child, log, label, forked, started: std::time::Instant::now(), _claim: None, triage: None }
    }

    /// Ended: what it said (`claude -p`'s JSON), and, a triage's, its answer kept in the issue's
    /// file (as dv issues' are).
    pub(super) fn kept(&self) -> (Option<serde_json::Value>, Option<Result<String>>) {
        let out: Option<serde_json::Value> = std::fs::read_to_string(&self.log).ok().and_then(|t| serde_json::from_str(&t).ok());
        let ok = out.as_ref().is_some_and(|v| v["is_error"] == false);
        let saved = match (&self.triage, &out) {
            (Some(i), Some(v)) if ok => Some(crate::list::issues::save_triage(i.number, &i.updated, &i.fixes, v)),
            _ => None,
        };
        (out, saved)
    }
}

/// On a thread waiting for Claude: what you want of it (`[change]` …), when you said.
pub(crate) fn want_tag(t: &Thread) -> Option<&'static str> {
    let tag = ask::WANTS.iter().find(|w| w.0 == t.want)?.1;
    t.waiting().then_some(tag)
}

/// Whose turn it is on an open thread of the review file, as its tag: `[waiting]` (Claude's),
/// `[answered]` (yours).
pub(super) fn turn(t: &Thread) -> Option<&'static str> {
    match (t.waiting(), t.answered()) {
        (true, _) => Some("[waiting] "),
        (_, true) => Some("[answered] "),
        _ => None,
    }
}

/// Claude's review for this PR/branch, and the background `claude -p` runs that write it.
#[derive(Default)]
pub(super) struct ClaudeState {
    /// How "make the changes" lands them: `""` uncommitted, `"amend"` into the commits they
    /// belong in, `"commit"` as new commits (`Space a` asks; the last answer comes first).
    pub(super) land: &'static str,
    /// Its review file (see `claude.rs`), and when that last changed.
    pub(super) path: Option<PathBuf>,
    pub(super) mtime: Option<std::time::SystemTime>,
    /// Why its file can't be read now (Claude mid-edit, a slip in its JSON): what was last read
    /// of it stays shown meanwhile.
    pub(super) broken: Option<String>,
    pub(super) summary: String,
    /// When Claude wrote its review.
    pub(super) created: String,
    /// The commit the review was made against (its `head`).
    pub(super) head: String,
    /// A new description for the PR that Claude proposes (empty: none).
    pub(super) description: String,
    /// A comment Claude drafted for closing the PR (empty: none).
    pub(super) closing: String,
    /// Why Claude would close it (empty: it wouldn't).
    pub(super) close: String,
    /// An issue's: Claude's triage of it.
    pub(super) triage: crate::claude::Triage,
    /// A background `claude -p` run: (process, log file, what it's doing).
    pub(super) job: Option<agent::ClaudeJob>,
    /// The Claude Code session of the last background run, resumed by the next.
    pub(super) session: Option<String>,
    /// A reply came in while Claude was busy: answer again when it's done.
    pub(super) again: bool,
    /// Background runs so far (each gets its own log).
    pub(super) runs: u32,
    /// The last background run's log (its result, as JSON).
    pub(super) log: Option<PathBuf>,
    /// What background runs have cost so far (as Claude Code reports it, in USD).
    pub(super) spent: f64,
    /// Another dv's Claude at work in this checkout on it (`watch_elsewhere`), and when that
    /// was last looked at.
    pub(super) elsewhere: Option<String>,
    /// This PR's replacement is being drafted (`watch_elsewhere`'s look).
    pub(super) replacing: Option<String>,
    pub(super) elsewhere_check: Option<std::time::Instant>,
    /// When Claude's pane was last looked at, and whether it was asking you something then.
    pub(super) pane_check: Option<std::time::Instant>,
    pub(super) asking: bool,
    /// Its pane shows it working (Claude Code's "esc to interrupt").
    pub(super) pane_busy: bool,
}

/// A triage run's label (`poll_claude` keeps its answer).
pub(super) const TRIAGE_LABEL: &str = "triaging it";

/// A view gone with a Claude run of its own still under way (quit, or a nested issue's view
/// dropped as a PR's link was followed): its end waited for elsewhere, a triage's answer kept,
/// its hold let go (dv asks before quitting while it holds one: `claude::claims_held`).
impl Drop for ClaudeState {
    fn drop(&mut self) {
        if let Some(mut j) = self.job.take() {
            std::thread::spawn(move || {
                _ = j.child.wait();
                _ = j.kept();
            });
        }
    }
}

/// How "make the changes" can land them: the key (`ClaudeState::land`), the picker's item.
/// How changes on someone else's PR may land too: as suggestions for you to post (`Ask::Fix`).
pub(super) const SUGGEST: &str = "suggest";

pub(super) const LANDS: [(&str, &str); 3] = [
    ("", "Leave them uncommitted (you commit them)"),
    ("amend", "Amend each into the commit it belongs in (dv's amend: a backup branch first)"),
    ("commit", "As new commits"),
];

impl App {
    /// Whether `ask` may change the code (or reads it whole: a review): answering does when a
    /// reply waiting wants a change.
    /// (An issue has no code: nothing changes it.)
    pub(super) fn edits(&self, ask: Ask) -> bool {
        match ask {
            _ if !self.has_code() => false,
            Ask::Describe | Ask::Close => false,
            Ask::Answer => self.threads.iter().any(|t| t.waiting() && super::ask::edits_code(&t.want)),
            _ => true,
        }
    }

    /// Whether `ask`'s run may commit (and amend): what makes changes you asked for, answering
    /// your replies too (their `[amend]`, `[commit]`), wherever it runs. The one rule.
    pub(super) fn commits(&self, ask: Ask) -> bool {
        self.has_code() && matches!(ask, Ask::Fix | Ask::Edit | Ask::Answer)
    }

    /// Replies of yours that Claude hasn't answered yet.
    pub(super) fn waiting_replies(&self) -> usize {
        self.threads.iter().filter(|t| t.waiting()).count()
    }

    /// The commit Claude last reviewed, if there's been more since that build on it (a push,
    /// your commits): then an update need only look at those. Not after a rebase or force-push:
    /// that's all new.
    pub(super) fn reviewed_before(&self) -> Option<&str> {
        let old = self.claude.head.as_str();
        let head = diff::git(&["rev-parse", "HEAD"]).ok()?;
        let newer = !old.is_empty() && !head.trim().starts_with(old);
        (newer && diff::git(&["merge-base", "--is-ancestor", old, "HEAD"]).is_ok()).then_some(old)
    }

    /// What "make the changes" acts on: the review file's open comments, Claude's and the ones
    /// you left it (`a`), not resolved or outdated. The one place that's decided.
    pub(super) fn change_requests(&self) -> usize {
        self.threads.iter().filter(|t| t.claude.is_some() && !t.stale() && (t.by_claude() || !matches!(t.want.as_str(), "ask" | "suggest"))).count()
    }

    /// Why Claude can't take a request right now (checked before asking you for instructions,
    /// so nothing you write is lost to it): still busy, or a rebase stopped (`pane_cant`: where
    /// it goes to the pane). `ask`: what's asked (`None`: amending, which changes the code but needs no review file).
    /// What changes the code (or reads it whole: a review) waits for a stopped rebase, whose
    /// conflicts are in the way; a question, a description, a closing comment don't.
    pub(super) fn claude_blocked(&self, ask: Option<Ask>) -> Option<String> {
        let edits = ask.is_none_or(|a| self.edits(a));
        // (Its pane working is no bar: what you send there waits in its input, as Claude Code
        // has it.)
        if self.claude.job.is_some() {
            return Some(if ask.is_none() { App::busy_edit("amend") } else { App::busy_says("ask Claude this") });
        }
        // At it from elsewhere (dv list's, dv issues' runs): one at a time on it, as here.
        if let Some(what) = self.held() {
            return Some(format!("Claude is {what} on {} already (elsewhere): ask again once it's done", self.what()));
        }
        // A rebase stopped part way: the tree has its conflicts, HEAD is detached.
        if edits && super::rebase::rebasing(&self.root) {
            return Some(super::rebase::stopped_says(&self.root).into());
        }
        None
    }

    /// Claude is at work in this checkout: a run of this dv's, its pane working, or another's
    /// on it (a replacement's redraft too: its claim). Nothing here rewrites the branch or commits under
    /// it meanwhile.
    /// (Another dv's run in this checkout too, dv list's say: looked at now.)
    pub(super) fn claude_busy(&self) -> bool {
        let here = || self.run_key().and_then(|k| crate::claude::at_work(&k, self.claude.session.as_deref(), Some(&self.root))).is_some();
        self.claude.job.is_some() || self.claude.pane_busy || here()
    }

    /// Why a redraft of draft `id` waits: Claude at work on it, from any dv (its draft, a run, a
    /// pane open on it: the redraft's claim waits for each).
    pub(super) fn redraft_waits(&self, id: u64) -> Option<String> {
        let w = crate::replace::load(id).ok()?.at_work()?;
        Some(format!("Claude is {w} on {}: Space a → Redraft once it's done", crate::replace::label(id)))
    }

    /// Claude at work on it (a run of this view's, or another's from any dv: a redraft, a pane
    /// of another session); its own pane, idle or not, isn't: what's sent there waits in it.
    pub(super) fn at_it(&self) -> bool {
        self.claude.job.is_some() || self.held().is_some()
    }

    /// What another run holds this PR (or issue) for, from any dv, any checkout (one at a time
    /// on it: a new one waits, a pane opened forks).
    pub(super) fn held(&self) -> Option<String> {
        self.run_key().and_then(|k| crate::claude::at_work(&k, self.claude.session.as_deref(), None))
    }

    /// Another dv's Claude at work in this checkout on it (dv list's run, a pane of another
    /// session), every few seconds: what the status bar's badge says.
    pub(super) fn watch_elsewhere(&mut self) {
        if self.claude.elsewhere_check.is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(3)) {
            return;
        }
        self.claude.elsewhere_check = Some(std::time::Instant::now());
        // (Not a run of this view's own: its job's claim.)
        let mine = self.claude.job.is_some();
        self.claude.elsewhere = self.run_key().filter(|_| !mine).and_then(|k| crate::claude::at_work(&k, self.claude.session.as_deref(), Some(&self.root)));
        // (Its replacement's draft, for the badge: not looked up every frame.)
        self.claude.replacing = self.replaced_by.as_ref().and_then(|r| r.at_work());
    }

    /// Why `ask` can't go to the PR's Claude pane, where it would (one is open): the pane is
    /// from an older dv that can't reach the review files, and `ask` needs them. (A run of its
    /// own, as answering is, reaches them whatever the pane.)
    fn pane_cant(&self, ask: Ask) -> Option<String> {
        let needs_files = !matches!(ask, Ask::Fix | Ask::Edit) || self.change_requests() > 0;
        let (_, cmd) = self.claude_pane_cmd()?;
        let dv = crate::claude::dv_dir().map(|d| d.display().to_string()).unwrap_or_default();
        (needs_files && !dv.is_empty() && !cmd.contains(&dv)).then(|| {
            format!("Claude's pane for {} can't reach dv's review files (it's from an older dv): close it, and Space a starts a new one", self.what())
        })
    }

    pub(super) fn claude_menu(&mut self) {
        if self.claude.path.is_none() {
            return self.msg = "no review file here (not in a git repo?)".into();
        }
        let has_review = self.threads.iter().any(Thread::by_claude) || !self.claude.summary.is_empty();
        // (An issue: triaged, its code nowhere to review or change.)
        let review = match (has_review, self.has_code()) {
            (true, false) => (format!("Triage {} again (your talk with Claude stays)", self.what()), Ask::Triage),
            (false, false) => (format!("Triage {}", self.what()), Ask::Triage),
            (true, _) => match self.reviewed_before() {
                Some(old) => (format!("Update Claude's review: what changed since it reviewed {}", old.get(..8).unwrap_or(old)), Ask::Review),
                None => ("Update Claude's review (keeps comments that still apply, and your replies)".into(), Ask::Review),
            },
            (false, _) => ("Review this".into(), Ask::Review),
        };
        let n = self.waiting_replies();
        let open = self.change_requests();
        let notes = match crate::claude::notes_path().is_ok_and(|p| p.exists()) {
            true => "Edit Claude's notes on this repo (it reads them before each task)",
            false => "Write notes for Claude on this repo (it reads them before each task)",
        };
        let mut items = vec![Item::new(review.0, Target::Claude(review.1)), Item::new(format!("Answer your replies ({n} waiting)"), Target::Claude(Ask::Answer))];
        // (How: suggested, on someone else's PR, or made here; and how they land.)
        if self.has_code() {
            items.push(Item::new(
                match open {
                    0 => "Make a change… (no open comments: you say what)".to_owned(),
                    n => format!("Make the changes… ({n} open comments)"),
                },
                Target::Claude(Ask::Fix),
            ));
        }
        // Your PR's CI failing: Claude finds out why; or found it isn't the PR's doing, and had
        // a fix of its own in mind, a new PR's.
        if let Some(n) = self.pr.as_ref().map(|p| p.number).filter(|_| self.failing > 0 && !self.theirs) {
            let s = if self.failing == 1 { "" } else { "s" };
            items.push(Item::new(format!("Fix the failing CI ({} check{s}: it reads the failed runs' logs; you push)", self.failing), Target::Claude(Ask::FixChecks)));
            if let Some(c) = self.ci_note(n).filter(|c| !c.task.is_empty()) {
                items.insert(0, Item::new(format!("Draft a PR fixing CI (not this PR's doing: {})…", c.why), Target::Do(Action::CiFixPr)));
            }
        }
        items.push(Item::new(format!("Ask a question about {}… (it answers beside its description)", self.what()), Target::ClaudeQuestion));
        items.extend([
            Item::new(notes, Target::ClaudeNotes),
            Item::new("Check Claude's review now (dv watches it; this reads it again)", Target::ClaudeCheck),
        ]);
        if self.claude_pane().is_some() {
            items.insert(0, Item::new(format!("Show Claude's pane (its session on {})", self.what()), Target::ClaudePane));
        } else if in_zellij() {
            let held = self.run_key().and_then(|k| crate::claude::run_held(&k));
            let what = match self.claude_busy() || held.is_some() {
                // (Its own session's at work: a copy of it, for you; what's said there stays there.)
                true => "a copy of its conversation to talk in: the original's at work, and won't hear it",
                false => &format!("its session on {}, to talk to it", self.what()),
            };
            items.push(Item::new(format!("Open Claude in a pane ({what})"), Target::ClaudeOpen));
        }
        // A description Claude proposed: yours to approve (posted then), take further, or drop.
        if self.has_proposal() {
            items.insert(0, Item::new("Discard Claude's proposed description", Target::DescDiscard));
            items.insert(0, Item::new("More changes to the description…", Target::Claude(Ask::Describe)));
            items.insert(0, Item::new("Approve Claude's description… (it replaces the one on GitHub)", Target::DescApprove(false)));
        } else if self.subject().is_some() {
            items.push(Item::new(format!("Change {}'s description… (you approve it)", self.what()), Target::Claude(Ask::Describe)));
        }
        // Closing it: Claude drafts the comment; you edit it, then close with it (or don't).
        // (Closed or merged meanwhile: the draft can only go.)
        let open = self.pr_state.is_some_and(|s| matches!(s.0, "OPEN" | "DRAFT"));
        let drafted = !self.claude.closing.trim().is_empty();
        if drafted {
            items.insert(0, Item::new("Discard Claude's closing comment", Target::CloseDiscard));
        }
        if drafted && open {
            items.insert(0, Item::new("Changes to the closing comment…", Target::Claude(Ask::Close)));
            items.insert(0, Item::new(format!("Close {}{} with Claude's comment (your editor first)…", self.what(), self.closes_as()), Target::Do(Action::ClosePr)));
        } else if self.subject().is_some() && open {
            items.push(Item::new(format!("Draft a comment closing {}… (you edit it, then close)", self.what()), Target::Claude(Ask::Close)));
        }
        // One that seems stuck: it can be stopped (the session stays, so the next run resumes it).
        if let Some(j) = &self.claude.job {
            items.insert(0, Item::new(format!("Stop Claude ({} for {})", j.label, ago(j.started)), Target::ClaudeStop));
        }
        let outdated = self.outdated_claude_comments().len();
        if outdated > 0 {
            items.push(Item::new(format!("Dismiss Claude's outdated comments ({outdated})"), Target::ClaudeDismissOutdated(false)));
        }
        if self.claude.session.is_some() {
            items.push(Item::new(format!("Start a fresh Claude session for {} (the review stays)", self.what()), Target::ClaudeFresh));
        }
        // A replacement: Claude writes the change itself, unpublished, for you to review here.
        if let Some(r) = &self.replaced_by {
            let how = r.at_work().map_or("to review".into(), |w| format!("Claude is {w} on it: dv shows it as it goes"));
            items.push(Item::new(format!("Open its replacement, {} ({how}; q comes back here)", crate::replace::label(r.id)), Target::OpenReplacement(r.id)));
        }
        match (&self.pr, &self.replacement) {
            // Its replacement, not published: redrafted (all it replaces); published, a PR like
            // any, Claude asked there.
            (Some(_), _) if let Some(r) = &self.replaced_by => {
                if let Some(why) = self.redraft_waits(r.id).filter(|_| r.published.is_none()) {
                    items.push(Item::new(why, Target::Note("")));
                } else if r.published.is_none() {
                    items.push(Item::new(format!("Redraft {} (Claude carries on, with your instructions)", crate::replace::label(r.id)), Target::Do(Action::Replace(r.ids(), None))));
                }
            }
            (Some(p), _) => items.push(Item::new(
                format!("Draft a replacement for #{} (Claude writes it properly, unpublished, in the background)", p.number),
                Target::Do(Action::Replace(vec![p.number], None)),
            )),
            (None, Some(r)) if let Some(why) = self.redraft_waits(r.id) => items.push(Item::new(why, Target::Note(""))),
            (None, Some(r)) => items.push(Item::new(
                "Redraft this (Claude carries on, with your instructions)",
                Target::Do(Action::Replace(r.ids(), None)),
            )),
            // An issue: a PR fixing it, Claude's draft (your editor first, on what it should do).
            _ => {
                if let Some(i) = &self.issue {
                    // (Those drafted already, to review: a PR fixing it, unpublished.)
                    for r in crate::replace::all().into_iter().filter(|r| r.published.is_none() && crate::replace::fixes(&r.task) == Some(i.number)) {
                        let how = r.at_work().map_or("to review".into(), |w| format!("Claude is {w} on it: dv shows it as it goes"));
                        items.push(Item::new(format!("Open its draft fix, {} ({how}; q comes back here)", crate::replace::label(r.id)), Target::OpenReplacement(r.id)));
                    }
                    let task = format!("Fix #{}: {}", i.number, i.title);
                    // (Saved as it is: Claude reads the issue, its talk and its triage itself.)
                    items.push(Item::new(format!("Draft a PR fixing #{}… (Claude writes it, unpublished, in the background)", i.number), Target::Do(Action::NewPr { prefill: task, as_is: true })));
                }
            }
        }
        // Preselected: answering your replies if any wait, else the review (never Stop: Enter
        // out of habit mustn't end a run).
        let want = |t: &Target| matches!(t, Target::Claude(a) if matches!((a, n > 0), (Ask::Answer, true) | (Ask::Review | Ask::Triage, false)));
        self.picker = Some(Picker::new(" claude ", items).select(want));
    }

    /// `Space a` → check now: the review file read again (as if it had changed), and the
    /// background run's state, rather than waiting for dv to notice.
    pub(super) fn check_claude(&mut self) {
        // Read it, changed or not (or gone: `None` would look unchanged then).
        self.claude.mtime = Some(std::time::SystemTime::UNIX_EPOCH);
        self.watch_claude();
        let ran = self.claude.job.is_some();
        self.poll_claude(); // a run that's ended says how (that's the news, then)
        if let Some(j) = &self.claude.job {
            self.msg = format!("Claude is still {} ({} so far); Space a can stop it", j.label, ago(j.started));
        } else if !ran && self.claude.summary.trim().is_empty() && !self.threads.iter().any(|t| t.claude.is_some()) {
            let ask = if self.has_code() { "review" } else { "triage" };
            self.msg = format!("Claude hasn't written a review yet (Space a → {ask} asks for one)");
        }
    }

    /// `Space a` → stop: end the background run (its log stays). The session is kept, so the
    /// next run resumes where it left off.
    pub(super) fn stop_claude(&mut self) {
        let Some(mut j) = self.claude.job.take() else { return self.msg = "Claude isn't working on anything".into() };
        // Its process group: Claude and the commands it's running (a build, the tests).
        let group = format!("-{}", j.child.id());
        _ = std::process::Command::new("kill").args(["-KILL", "--", &group]).stderr(std::process::Stdio::null()).status();
        _ = j.child.kill();
        _ = j.child.wait();
        self.claude.again = false;
        self.msg = format!("stopped Claude ({} for {}); its log: {} (and .err)", j.label, ago(j.started), j.log.display());
        self.reload_tree(); // what it changed before it stopped
    }

    /// Forget the PR's Claude session, so the next run starts a new one: when it has got
    /// something wrong into its head, or grown long. The review file is what carries over.
    pub(super) fn fresh_claude_session(&mut self) {
        if self.claude.job.is_some() {
            return self.msg = App::busy_says("start a fresh session");
        }
        let pane = self.claude_pane().is_some();
        self.set_session(None);
        self.msg = match pane {
            true => "the next Claude run starts a new session; close Claude's pane, which has the old one".into(),
            false => "the next Claude run starts a new session".into(),
        };
    }

    /// `Space a` → approve Claude's description: after a confirmation (`sure`), it replaces
    /// the PR's on GitHub, and the proposal goes.
    pub(super) fn approve_description(&mut self, sure: bool) {
        let (Some(pr), Some(path)) = (self.subject().map(|s| s.n()), self.claude.path.clone()) else { return };
        if !sure {
            let items = vec![
                Item::new("No, not yet", Target::Nothing),
                Item::new(format!("Yes, replace #{pr}'s description on GitHub with Claude's"), Target::DescApprove(true)),
            ];
            return self.picker = Some(Picker::new(" description ", items));
        }
        // (A new title of Claude's too, if it has one: an issue's; else only the text.)
        let (text, title) = (self.proposed_text(), self.proposed_title());
        let body = (0..self.info.len()).find(|&i| self.is_description(i)).map(|h| h + 1).filter(|&b| self.info.get(b).is_some_and(|l| l.md));
        let shown = body.map_or(String::new(), |b| self.info[b].text.clone());
        // Edited on GitHub since dv showed it: the proposal's diff was against an older text.
        let now = match crate::gh::text(pr) {
            Ok(t) => t,
            Err(e) => return self.msg = format!("description not changed: {e:#}"),
        };
        if now.1.trim() != shown.trim() || title.is_some() && now.0 != self.title_now() {
            if let Some(b) = body {
                self.info[b].text = now.1;
            }
            let keep = self.cursor_key();
            self.rebuild(keep);
            return self.msg = "description not changed: it was edited on GitHub since; dv shows it now against Claude's: approve again if it's still right".into();
        }
        if let Err(e) = crate::gh::set_text(pr, title.as_deref(), &text) {
            return self.msg = format!("description not changed: {e:#}");
        }
        self.github_changed();
        // What's on GitHub now, without waiting for the next fetch.
        if let Some(b) = body {
            self.info[b].text = text;
        }
        if let (Some(t), Some(i)) = (title, &mut self.issue) {
            i.title = t;
        }
        let cleared = crate::claude::clear(&path, "description").and_then(|_| crate::claude::clear(&path, "title"));
        let keep = self.cursor_key();
        self.load_claude();
        self.rebuild(keep);
        self.msg = match cleared {
            Ok(()) => format!("#{pr}'s description is Claude's now"),
            Err(e) => format!("#{pr}'s description is Claude's now, but its proposal is still in the review file: {e:#} (Space a discards it)"),
        };
    }

    /// How an issue closes, as Claude's triage says (" as a duplicate of #12"); a PR's, nothing.
    fn closes_as(&self) -> String {
        let t = &self.claude.triage;
        match self.issue.is_some() {
            true => format!(" {}", crate::gh::close_how(&t.reason, t.duplicate_of).1),
            false => String::new(),
        }
    }

    /// `Space a` → close the PR with Claude's comment: in your editor first (an empty one, or
    /// `:cq`, closes nothing), then posted, and the PR closed.
    pub(super) fn close_pr(&mut self) -> Result<String> {
        let (Some(s), Some(path)) = (self.subject(), self.claude.path.clone()) else { return Ok(String::new()) };
        let n = s.n();
        let what = format!("comment closing #{n} (posted on its conversation, then it's closed{}; empty closes nothing)", self.closes_as());
        let Some(text) = compose_with(&what, false, self.claude.closing.trim(), true)?.filter(|t| !t.trim().is_empty()) else {
            return Ok(format!("#{n} not closed"));
        };
        let t = &self.claude.triage;
        // (Its state as GitHub now has it: an issue's by why it closed.)
        let closed = match s {
            crate::claude::Subject::Pr(_) => crate::gh::close(n, Some(text.trim())).map(|_| serde_json::json!({ "state": "CLOSED" })),
            crate::claude::Subject::Issue(_) => crate::gh::close_issue(n, text.trim(), &t.reason, t.duplicate_of)
                .map(|why| serde_json::json!({ "__typename": "Issue", "state": "CLOSED", "stateReason": why })),
        };
        let now = keep_on_error(closed, &text)?;
        _ = crate::claude::clear(&path, "closing");
        self.pr_state = Some(super::info::state(&now));
        self.github_changed();
        let keep = self.cursor_key();
        self.load_claude();
        self.rebuild(keep);
        Ok(format!("#{n} closed, with your comment"))
    }

    /// `Space a` → discard Claude's draft (`keys`: `"description"` and `"title"`, `"closing"`),
    /// `what` it is.
    pub(super) fn discard(&mut self, keys: &[&str], what: &str) {
        let Some(path) = self.claude.path.clone() else { return };
        self.msg = match keys.iter().try_for_each(|k| crate::claude::clear(&path, k)) {
            Ok(()) => {
                let keep = self.cursor_key();
                self.load_claude();
                self.rebuild(keep);
                format!("{what} discarded")
            }
            Err(e) => format!("{e:#}"),
        };
    }

    /// The ids of Claude's own comments whose code has changed (not your comments for it).
    pub(super) fn outdated_claude_comments(&self) -> Vec<String> {
        self.threads.iter().filter(|t| t.by_claude() && t.outdated).filter_map(|t| t.claude.clone()).collect()
    }

    /// From the `Space a` menu: dismiss every outdated comment of Claude's, after a confirmation
    /// (`sure`: the "Yes" in it).
    pub(super) fn dismiss_outdated(&mut self, sure: bool) {
        let keys = self.outdated_claude_comments();
        if keys.is_empty() {
            return self.msg = "no outdated comments from Claude".into();
        }
        if !sure {
            let items = vec![
                Item::new("No, keep them", Target::Nothing),
                Item::new(format!("Yes, dismiss Claude's {} outdated comment(s)", keys.len()), Target::ClaudeDismissOutdated(true)),
            ];
            return self.picker = Some(Picker::new(" dismiss outdated? ", items));
        }
        let Some(p) = self.claude.path.clone() else { return };
        let keep = self.cursor_key();
        let res = crate::claude::dismiss_all(&p, &keys);
        self.load_claude();
        self.rebuild(keep);
        self.msg = match res {
            Ok(n) => format!("dismissed {n} outdated comment(s) from Claude"),
            Err(e) => format!("{e:#}"),
        };
    }

    /// `extra`: the user's own instructions for this run (may be empty).
    pub(super) fn claude_prompt(&self, ask: Ask, extra: &str) -> String {
        let what = self.subject().map_or("the current branch".into(), |s| s.what());
        let prompt = match ask {
            Ask::Review => match self.reviewed_before() {
                Some(old) => update_prompt(&what, old),
                None => review_prompt(&what),
            },
            Ask::Answer => answer_prompt(&what, self.theirs),
            Ask::FixChecks => fix_checks_prompt(self.pr.as_ref().map_or(0, |p| p.number)),
            Ask::Close => format!(
                "Use the dv-review skill to draft a comment closing {what}, as the user asks below, in the review file's \
                 top-level \"closing\" field (see \"Draft a closing comment\"). Start from the draft already there, if there \
                 is one. Don't close it or comment on GitHub: the user closes it in dv."
            ),
            Ask::Describe => format!(
                "Use the dv-review skill to propose a new description for {what}, as the user asks below: the whole new text, \
                 in the review file's top-level \"description\" field (see \"Propose a description\"). Start from the proposal \
                 already there, if there is one, else from its current description. Don't change it on GitHub: the user \
                 approves it in dv."
            ),
            Ask::Triage => unreachable!("an issue's triage is its own run (`run_claude`)"),
            Ask::Fix if self.theirs && self.change_requests() == 0 => format!(
                "Use the dv-review skill to suggest the change the user asks for below, in the dv review of {what}: \
                 it's someone else's PR, so don't edit the code; add a comment of yours there with a suggestion block."
            ),
            Ask::Fix if self.theirs => format!(
                "Use the dv-review skill to suggest the changes the open comments in the dv review of {what} ask for \
                 (see \"Suggest the changes\"): it's someone else's PR, so don't edit the code."
            ),
            // Making them (yours, or someone else's when you ask Claude to all the same).
            Ask::Fix | Ask::Edit if self.change_requests() == 0 => format!(
                "Make the change the user asks for below, in {what}. First run `dv review-info`: if the notes it lists \
                 exist, read them and follow them (they're the dv-review skill's notes on this repo)."
            ),
            Ask::Fix | Ask::Edit => format!(
                "Use the dv-review skill to make the changes the open comments in the dv review of {what} ask for \
                 (your own, and the user's: those with \"author\": \"user\"; every one that isn't resolved or outdated), \
                 replying on each with what you changed and resolving the ones you addressed."
            ),
        };
        // Making them: how they land, as you picked.
        let land = match ask {
            Ask::Fix if self.theirs => "",
            Ask::Fix | Ask::Edit => match self.claude.land {
                "amend" => " Amend each change into the commit of the branch it belongs in (`dv amend <sha> <files>`, as the dv-review skill says), and say in its reply which commit it went in; a comment whose `want` says otherwise, as it says.",
                "commit" => " Commit the changes as new commits (`git add`, `git commit`, messages in the repo's style): one for each comment, or one for those that belong together; say in each reply which commit has it; a comment whose `want` says otherwise, as it says.",
                _ => " Leave the changes uncommitted (the user commits them); a comment whose `want` says otherwise, as it says.",
            },
            _ => "",
        };
        // Claude Code knows the date but not the time, and makes one up for `created`.
        with_instructions(&format!("{prompt}{land} (It's {} now.)", crate::claude::now()), extra)
    }

    /// Claude's note that PR `n`'s failing CI isn't its doing, written at its head now.
    pub(super) fn ci_note(&self, n: u64) -> Option<crate::claude::CiNote> {
        crate::claude::ci_note(n, &self.pr.as_ref()?.head_oid)
    }

    /// Open Claude's notes on the repo in hx (they're plain markdown; you can write them too).
    pub(super) fn edit_claude_notes(&mut self) {
        match crate::claude::notes_path() {
            Ok(p) => {
                _ = std::fs::create_dir_all(p.parent().unwrap_or(&p));
                self.action = Some(Action::Open(p.display().to_string()));
            }
            Err(e) => self.msg = format!("{e:#}"),
        }
    }

    /// From the `Space a` menu. Reviewing and making the changes first ask for your own
    /// instructions (an editor; optional, except for a change with no open comments to act on,
    /// where they're the request); answering replies just goes. Anything that would stop Claude
    /// taking it is checked first, so what you write isn't lost.
    pub(super) fn pick_claude(&mut self, ask: Ask) {
        // (Before the editor: nothing you write is lost to it.)
        // (A triage is never the pane's.)
        let pane = Some(ask).filter(|a| !matches!(a, Ask::Triage)).and_then(|a| self.pane_cant(a));
        if let Some(why) = self.claude_blocked(Some(ask)).or(pane) {
            return self.msg = why;
        }
        match ask {
            Ask::Describe | Ask::Close | Ask::Triage if self.subject().is_none() => self.msg = "no PR or issue here".into(),
            // Making the changes (not suggesting them): how they land, first.
            // Making the changes: how they land. On someone else's PR, suggested (for you to
            // post) or made here (`Edit`: for you to push to their branch, when they let
            // maintainers: else they stay here).
            Ask::Fix | Ask::Edit => {
                let here = if self.theirs { Ask::Edit } else { Ask::Fix };
                let mut items: Vec<Item> = LANDS.iter().map(|&(key, label)| Item::new(label, Target::Land(here, key))).collect();
                if self.theirs {
                    let push = match self.pr.as_ref().is_some_and(|p| p.maintainer_can_modify || !p.cross_repo) {
                        true => "Space g pushes them to their branch",
                        false => "they don't allow maintainer edits: the commits stay here",
                    };
                    items.iter_mut().for_each(|i| i.label = format!("{} ({push})", i.label));
                    items.insert(0, Item::new("As suggestions for you to post (their code stays as it is)", Target::Land(Ask::Fix, SUGGEST)));
                }
                // (Their PR: suggested, unless you've said how; made here, Space g pushes
                // commits, so as commits.)
                let land = match (self.theirs, self.claude.land) {
                    (true, "") => SUGGEST,
                    (_, l) => l,
                };
                let p = Picker::new(" how should the changes land? ", items).select(|t| matches!(t, Target::Land(_, k) if *k == land));
                self.picker = Some(p);
            }
            Ask::Review | Ask::Describe | Ask::Close | Ask::Triage => self.action = Some(Action::ClaudeWith(ask)),
            ask => _ = self.run_claude(ask, ""),
        }
    }

    /// How the changes land picked (`Space a` → make the changes): on to your instructions.
    pub(super) fn land(&mut self, ask: Ask, land: &'static str) {
        if land != SUGGEST {
            self.claude.land = land;
        }
        self.action = Some(Action::ClaudeWith(ask));
    }

    /// Ask Claude (`extra`: your instructions, may be empty). Whether it was asked: if not,
    /// `msg` says why.
    pub(super) fn run_claude(&mut self, ask: Ask, extra: &str) -> bool {
        if matches!(ask, Ask::Answer) && self.waiting_replies() == 0 {
            self.msg = "no replies waiting for Claude (r on a Claude comment replies)".into();
            return false;
        }
        let requests = self.change_requests();
        if matches!(ask, Ask::Fix | Ask::Edit) && requests == 0 && extra.trim().is_empty() {
            self.msg = "nothing to change: no open comments, and no instructions written".into();
            return false;
        }
        if let Some(why) = self.claude_blocked(Some(ask)) {
            self.msg = why;
            return false;
        }
        let label = match ask {
            Ask::Review => "reviewing",
            Ask::Answer => "answering your replies",
            Ask::Describe => "proposing a description",
            Ask::Fix if self.theirs => "suggesting the changes",
            Ask::Edit => "making the changes",
            Ask::Close => "drafting a closing comment",
            Ask::Fix => "making the changes",
            Ask::FixChecks => "fixing CI",
            // An issue's triage: dv issues' own run, in the background (not a pane's: reading
            // only, its answer merged into the file as it ends, `ClaudeJob::kept`).
            Ask::Triage => {
                let Some(i) = self.issue.clone() else { return false };
                let cmd = crate::list::issues::triage_command(i.number, &i.fixes, &self.root, extra);
                self.msg = match self.spawn_job(cmd, TRIAGE_LABEL, false) {
                    Ok(job) => {
                        let n = i.number;
                        self.claude.job = Some(ClaudeJob { triage: Some(i), ..job });
                        format!("Claude is triaging #{n}…")
                    }
                    Err(e) => format!("{e:#}"),
                };
                return self.claude.job.is_some();
            }
        };
        let prompt = self.claude_prompt(ask, extra);
        // Claude already open on this PR in a pane: put the request there, rather than open
        // another. Pasted, not sent: what the pane is showing (a permission prompt, your own
        // half-typed text) isn't dv's to answer, so you press Enter there.
        if let Some(pane) = self.claude_pane() {
            if let Some(why) = self.pane_cant(ask) {
                self.msg = why;
                return false;
            }
            let (msg, asked) = match crate::pane::offer(pane, &prompt) {
                Ok(true) => (format!("{label}: the request is in Claude's pane, press Enter there to send it"), true),
                Ok(false) => ("Claude's pane is waiting for you to answer it; ask again after".into(), false),
                Err(e) => (format!("Claude's pane: {e:#}"), false),
            };
            self.msg = msg;
            return asked;
        }
        let ci = self.pr.as_ref().filter(|_| matches!(ask, Ask::FixChecks)).map(|p| p.number);
        let more = tools_for(ci, self.commits(ask));
        let (msg, asked) = match in_zellij() {
            // A real session in a pane (the PR's, as the background runs use): watch it, approve
            // what it asks, talk to it.
            // Floating and unfocused: hidden while zellij's floating panes are (`Space a` →
            // show it, or zellij's own toggle); dv says when it's asking you something, and
            // closes it once Claude's done there (`close_if_done`).
            true => match self.claude_pane_run(&format!("claude: {label}"), Some(&prompt), Some(ask.work()), false, false) {
                Ok(()) => {
                    (format!("Claude is {label} in a hidden pane (Space a shows it; it closes when Claude's done); its comments show up here as it writes them"), true)
                }
                Err(e) => (format!("{e:#}"), false),
            },
            false => match self.spawn_claude(&prompt, label, ask.work(), false, &more.iter().map(String::as_str).collect::<Vec<_>>()) {
                Ok(job) => {
                    self.claude.job = Some(job);
                    (format!("Claude is {label} in the background…"), true)
                }
                Err(e) => (format!("starting claude: {e:#}"), false),
            },
        };
        self.msg = msg;
        asked
    }

    /// After `r` on Claude's comment (or `a`): have Claude answer right away, in the
    /// background (also in zellij: a pane per reply would get in the way). If the PR's session
    /// is open in a pane, this runs on a fork of it: the same conversation so far, without two
    /// processes on one session. Replies made while it's working get one more run when it
    /// finishes. Returns what to tell the user.
    pub(super) fn answer_now(&mut self) -> String {
        // What was just written (what it waits on, and wants): read, and shown (one goes with
        // the other: the rows point into the threads).
        let keep = self.cursor_key();
        self.load_claude();
        self.rebuild(keep);
        if self.claude.job.is_some() {
            self.claude.again = true;
            return "reply saved; Claude answers it when it's done with what it's doing".into();
        }
        if let Some(why) = self.claude_blocked(Some(Ask::Answer)) {
            return format!("reply saved; {why} (then Space a has Claude answer)");
        }
        // A change asked for while Claude's pane works in this checkout: not two at once on its
        // code (`edits`, the one rule).
        if self.claude.pane_busy && self.edits(Ask::Answer) {
            return "reply saved; Claude's pane is at work here, and this changes code: Space a has Claude answer once it's done".into();
        }
        let prompt = self.claude_prompt(Ask::Answer, "");
        let forked = self.claude_pane().is_some();
        let more = tools_for(None, self.commits(Ask::Answer));
        match self.spawn_claude(&prompt, "answering", Ask::Answer.work(), forked, &more.iter().map(String::as_str).collect::<Vec<_>>()) {
            Ok(job) => {
                self.claude.job = Some(job);
                "reply sent; Claude is answering…".into()
            }
            Err(e) => format!("reply saved, but starting claude failed: {e:#} (Space a to retry)"),
        }
    }

    /// The zellij pane running this PR's Claude Code session, if one is open: dv starts those
    /// with the session id on the command line, which no other pane has.
    pub(super) fn claude_pane(&self) -> Option<u64> {
        self.claude_pane_cmd().map(|p| p.0)
    }

    /// The PR's session (`fork`: a fork of it) in a floating pane (`prompt`: sent as it starts):
    /// hidden (`focus`: shown and focused). Floating and unfocused, it's hidden while zellij's floating panes are
    /// (`Space a` → show it, or zellij's own toggle); dv says when it's asking you something,
    /// and closes it once Claude's done there (`close_if_done`).
    fn claude_pane_run(&mut self, name: &str, prompt: Option<&str>, work: Option<crate::claude::Work>, focus: bool, fork: bool) -> Result<()> {
        // Its marker first, every pane on the session (not a fork: a session of its own), for dv
        // list and any dv (`claude::claim` waits while it's open). While a run holds it: a pane
        // to talk in forks (that run's at the session), a request's refused.
        // (Its session settled first: what it runs on, its marker names.)
        let mut session = self.session_args(fork);
        let marked = match (fork, self.run_key(), session[1].clone()) {
            (false, Some(k), id) => {
                let what = match prompt {
                    Some(_) => format!("{} (in its pane)", name.trim_start_matches("claude: ")),
                    None => "talking with you (in its pane)".into(),
                };
                match crate::claude::mark_pane(&k, &what, &id, prompt.is_none(), &self.root) {
                    Ok(()) => Some((k, id)),
                    Err(_) if prompt.is_none() => {
                        session = self.session_args(true);
                        None
                    }
                    Err(w) => anyhow::bail!("Claude is {w} on {} already (elsewhere): ask again once it's done", self.what()),
                }
            }
            _ => None,
        };
        let mut path_env = std::ffi::OsString::from("PATH=");
        path_env.push(path_for_claude());
        let mut cmd = std::process::Command::new("zellij");
        cmd.args(["run", "--floating", "--name", name, "--cwd"]).arg(&self.root);
        if !focus {
            cmd.arg("--no-focus");
        }
        cmd.args(["--", "env"])
            .arg(path_env) // the pane gets zellij's environment, not dv's
            .args(data_env()) // and so the same dv directory as this one, which it watches
            .args(self.subject().map(|s| s.env()).map(|(k, v)| format!("{k}={v}")))
            .arg("claude")
            .args(prompt)
            .args(crate::claude::model_args(work, &self.root)) // (none: `dv.model`, it's for anything)
            .args(&session)
            .args(claude_dirs())
            // (A checkout of its own to try things in, as the skill has it: not asked each time.)
            .args(["--allowedTools", "Bash(dv scratch:*)"]);
        let s = cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
        if let (false, Some((_, id))) = (s.as_ref().is_ok_and(|s| s.success()), &marked) {
            crate::claude::clear_pane(id);
        }
        let s = s.context("running zellij")?;
        anyhow::ensure!(s.success(), "zellij run failed ({s})");
        Ok(())
    }

    /// `Space a` → open Claude in a pane, to talk to it: the PR's session, or while a run is at
    /// work on it, a fork of it (two at once on one session would trip each other up).
    pub(super) fn open_claude_pane(&mut self) {
        let busy = self.claude_busy();
        if let Err(e) = self.claude_pane_run(if busy { "claude (a copy)" } else { "claude" }, None, None, true, busy) {
            self.msg = format!("{e:#}");
        }
    }

    /// `Space a` → show Claude's pane (`show_pane`).
    pub(super) fn show_claude_pane(&mut self) {
        let Some((pane, ..)) = self.claude_pane_cmd() else { return self.msg = "Claude's pane is closed".into() };
        if crate::pane::show(pane).is_err() {
            self.msg = "zellij couldn't show Claude's pane".into();
        }
    }

    /// Every few seconds: whether Claude's pane (hidden, maybe) is asking you something, a
    /// permission prompt it won't get past alone; said once each time it starts asking.
    pub(super) fn watch_claude_pane(&mut self) {
        if self.claude.session.is_none() || !in_zellij() {
            // No session (a fresh one asked for), no pane to watch: not busy, not asking.
            return (self.claude.pane_busy, self.claude.asking) = (false, false);
        }
        if self.claude.pane_check.is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(3)) {
            return;
        }
        self.claude.pane_check = Some(std::time::Instant::now());
        let pane = self.claude_pane();
        let screen = pane.map(crate::pane::screen);
        let asking_now = screen.as_deref().is_some_and(crate::pane::asking);
        if asking_now && !self.claude.asking {
            self.msg = "Claude is asking you something in its pane: Space a → Show Claude's pane".into();
        }
        self.claude.asking = asking_now;
        // Claude Code says how to stop it while it's working.
        self.claude.pane_busy = screen.as_deref().is_some_and(crate::pane::working);
        // Done there: its pane closes (`close_if_done`); the session stays, for the next ask.
        // (Its marker: this checkout's session's, not another's run.)
        let Some(pane) = pane else { return };
        let Some(session) = self.claude.session.clone() else { return };
        let Some(since) = crate::claude::pane_run(&session) else { return };
        if crate::pane::close_if_done(pane, &session, since) {
            crate::claude::clear_pane(&session);
            self.msg = "Claude's done: its pane closed (Space a asks it more, in the same session)".into();
        }
    }

    /// What Claude is doing here, for the status bar's badge: a run in the background, or its pane
    /// busy (or waiting on you). `None` when it's idle.
    pub(super) fn claude_doing(&self) -> Option<String> {
        match &self.claude.job {
            Some(j) => Some(format!("◐ claude: {} {}", j.label, ago(j.started))),
            None if self.claude.asking => Some("◐ claude is asking you (Space a)".into()),
            None if self.claude.pane_busy => Some("◐ claude working (its pane)".into()),
            None if let Some(w) = &self.claude.replacing => Some(format!("◐ its replacement: claude {w} (Space a: open it)")),
            None => self.claude.elsewhere.as_ref().map(|w| format!("◐ claude {w} (another dv's)")),
        }
    }

    /// Why `what` waits, Claude at work here (`claude_busy`): one wording, what you did first (a
    /// message cut short keeps that), the status bar's badge saying on what Claude is.
    pub(super) fn busy_says(what: &str) -> String {
        format!("{what} once Claude's done (it's at work here: the status bar says on what)")
    }

    /// `busy_says`, for what changes the code under Claude (amend, commit): why it waits too.
    pub(super) fn busy_edit(what: &str) -> String {
        App::busy_says(&format!("{what} (its changes would be half made)"))
    }

    /// The PR's Claude pane and its command line.
    fn claude_pane_cmd(&self) -> Option<(u64, String)> {
        let id = self.claude.session.as_ref().filter(|_| in_zellij())?;
        crate::pane::of_session(&crate::pane::open(), id).map(|p| (p.0, p.1.clone()))
    }

    /// The PR's Claude Code session in this checkout: resume it if Claude Code has it, else
    /// start it under a new id. The id is saved next to the review file (per checkout, as Claude
    /// Code keeps sessions per directory), so the next run continues the same conversation. One
    /// that never started (a pane that failed) isn't resumed: Claude Code doesn't have it.
    /// (`fork`: a fork of it, which leaves it as it is: of it, once Claude Code has it, else a
    /// throwaway session; neither saved, nor this view's.)
    fn session_args(&mut self, fork: bool) -> Vec<String> {
        if fork {
            return match self.claude.session.clone().filter(|id| crate::claude::session_exists(id)) {
                Some(id) => vec!["--resume".into(), id, "--fork-session".into()],
                None => vec!["--session-id".into(), crate::claude::new_session_id()],
            };
        }
        let args = match &self.claude.path {
            Some(p) => crate::claude::session_args(p, &self.root).ok(),
            None => None,
        };
        let args = args.unwrap_or_else(|| ["--session-id".into(), crate::claude::new_session_id()]);
        self.claude.session = Some(args[1].clone());
        args.to_vec()
    }

    /// Remember (or, with `None`, forget) the PR's session in this checkout, on disk too.
    pub(super) fn set_session(&mut self, id: Option<String>) {
        if let Some(p) = &self.claude.path {
            _ = crate::claude::save_session(p, &self.root, id.as_deref());
        }
        self.claude.session = id;
    }

    /// `claude -p` in the background: its result (JSON) in a log file, its errors next to it.
    /// It runs in the PR's session (or, `forked`, a fork of it), so Claude keeps the review's
    /// context across runs.
    /// `more`: tools it may use on top of `ALLOWED`.
    /// `work`: its model's (`dv.model.<work>`).
    pub(super) fn spawn_claude(&mut self, prompt: &str, label: &'static str, work: crate::claude::Work, forked: bool, more: &[&str]) -> Result<ClaudeJob> {
        let session = self.session_args(forked);
        self.spawn_job(claude_command(work, prompt, &self.root, &session, more, None), label, forked)
    }

    /// `cmd` (a Claude run) in the background, this view's (`label`: what it's doing; `forked`,
    /// as `spawn_claude`'s): its log, and its marks for dv list (and any dv) to see.
    pub(super) fn spawn_job(&mut self, mut cmd: std::process::Command, label: &'static str, forked: bool) -> Result<ClaudeJob> {
        use std::os::unix::process::CommandExt;
        // Its hold on it: one run at a time on it, a fork's too (its pane working: that run's).
        let mut claim = match self.run_key() {
            Some(k) => Some(crate::claude::claim(&k, label, self.claude.session.as_deref())?.map_err(|w| anyhow!("Claude is {w} on {} already (elsewhere): ask again once it's done", self.what()))?),
            None => None,
        };
        // One log per run, so a follow-up run doesn't wipe the one a message points to.
        self.claude.runs += 1;
        let log = std::env::temp_dir().join(format!("dv-claude-{}-{}.log", std::process::id(), self.claude.runs));
        let err = log.with_extension("err");
        let child = cmd
            .envs(self.subject().map(|s| s.env()))
            .stdin(std::process::Stdio::null())
            .stdout(std::fs::File::create(&log)?)
            .stderr(std::fs::File::create(&err)?)
            .process_group(0) // so stopping it stops what it's running too
            .spawn()
            .context("is Claude Code (`claude`) installed?")?;
        self.claude.log = Some(log.clone());
        if let Some(c) = &mut claim {
            c.set_pid(child.id());
            // (Its checkout: an issue's run has none of its own, nothing to say.)
            if self.has_code() {
                c.set_dir(&self.root);
            }
        }
        Ok(ClaudeJob { _claim: claim, ..ClaudeJob::new(child, log, label, forked) })
    }

    /// When a background run ends: keep its session for the next one, say how it went (its
    /// writes have already shown up via `watch_claude`), and answer replies made meanwhile.
    pub(super) fn poll_claude(&mut self) {
        let Some(job) = &mut self.claude.job else { return };
        let Ok(Some(st)) = job.child.try_wait() else { return };
        let Some(job) = self.claude.job.take() else { return };
        // (What it said, a triage's kept, before the file's read again; its hold let go.)
        let (out, triaged) = job.kept();
        // (Its hold let go first: answering what came in meanwhile, below, claims anew.)
        let ClaudeJob { log, label, forked, _claim, .. } = job;
        drop(_claim);
        let failed = !st.success() || out.as_ref().is_none_or(|v| v["is_error"] == true);
        self.reload_tree(); // it may have changed code (making changes, or answering a request)
        // A fork's id isn't the PR's session (that one's open in a pane): keep the PR's; a
        // triage's run is a reading one, of no session.
        match out.as_ref().and_then(|v| v["session_id"].as_str()) {
            _ if forked || label == TRIAGE_LABEL => {}
            Some(id) if !failed => self.set_session(Some(id.to_owned())),
            _ => self.set_session(None), // e.g. the session is gone: start afresh next time
        }
        // A run can end "successfully" having been refused what it needed: say what.
        let refused = out.as_ref().and_then(crate::claude::refusal);
        let cost = out.as_ref().and_then(|v| v["total_cost_usd"].as_f64());
        self.claude.spent += cost.unwrap_or(0.0);
        let took = match (out.as_ref().and_then(|v| v["num_turns"].as_u64()), cost) {
            (Some(n), Some(c)) => format!(" ({n} turns, ${c:.2})"),
            _ => String::new(),
        };
        self.msg = match (failed, &refused) {
            (false, None) => format!("Claude is done {label}{took}"),
            (false, Some(r)) => format!("Claude finished {label}, but was refused {r}: see {}", log.display()),
            (true, Some(r)) => format!("Claude stopped while {label}, refused {r}: see {}", log.display()),
            (true, None) => format!("Claude stopped ({st}) while {label}: see {} (and .err)", log.display()),
        };
        if let Some(Err(e)) = triaged {
            self.msg = format!("Claude's triage couldn't be kept: {e:#} (see {})", log.display());
        }
        // A rebase Claude carried on: how it stands now.
        if label == super::rebase::REBASE_LABEL {
            self.rebase_ended();
        }
        // Placing changes for an amend: its answers, in the picker.
        // (Not over an action on its way; a picker or prompt, watch() waits out.)
        // (Asked to place it all: amended, now.) Its answers are the file it filled in: a
        // command it was refused on the way (a `jq` to read it, say) doesn't stop that.
        let then = std::mem::take(&mut self.amend_then);
        if label == super::amend::AMEND_LABEL && !failed && then {
            self.amend_now();
        } else if label == super::amend::AMEND_LABEL && !failed {
            match self.action.is_none() {
                true => self.amend_menu(),
                false => self.msg += " (Space g → amend shows where it put them)",
            }
        }
        // The commit message it wrote: your editor opens with it (gone wrong, or not written:
        // the draft from its replies).
        if label == super::git::COMMIT_LABEL {
            let wrote = self.commit_msg_path().and_then(|p| std::fs::read_to_string(&p).ok()).map(|m| m.trim().to_owned()).filter(|m| !m.is_empty() && !failed);
            // (Not over what you're at: kept, for Space g → commit to open with.)
            match self.action.is_none() && self.writing.is_none() {
                true => self.action = Some(Action::CommitWith(wrote.unwrap_or_else(|| self.commit_draft()))),
                false => self.msg += " (Space g → commit opens with its message)",
            }
        }
        // Replies made meanwhile: one more run, unless this one went wrong (then that's the news,
        // and retrying would hit the same wall).
        if std::mem::take(&mut self.claude.again) && self.waiting_replies() > 0 {
            // (Not with a commit about to be made: the editor's open on it.)
            match failed || refused.is_some() || super::rebase::rebasing(&self.root) || label == super::git::COMMIT_LABEL {
                true => self.msg += " · your newer replies wait (Space a to retry)",
                false => self.msg = self.answer_now(),
            }
        }
    }

}

/// How long since `t`, for the status bar: "40s", "3m".
pub(super) fn ago(t: std::time::Instant) -> String {
    crate::gh::ago_secs(t.elapsed().as_secs() as i64)
}
