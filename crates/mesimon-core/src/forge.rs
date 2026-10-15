//! Pull requests, as the board reads them off a forge (T-739).
//!
//! The board already answers "is it merged?" from git (T-267); what git
//! cannot say is the PR itself, its pipeline and where it was deployed. The
//! daemon asks the forge CLI the person already signed in to — `gh` on a
//! github.com remote, `glab` on a GitLab host — reads only, one list call
//! per repository per sample, and this module is the pure half of that
//! road: the common `Pr` the two CLIs' JSON is read into, the one
//! `pipeline` word for GitHub's checks and GitLab's pipelines, the remote
//! URL parser that says which forge a repository is on, the rule that binds
//! a PR to a ticket, and the transcript detector that finds the URL a
//! `gh pr create` printed. No I/O here; the daemon's `forge.rs` runs the
//! commands and feeds the answers in.
//!
//! **A ticket's PRs are the PRs its work made**, never the ones its agent
//! mentioned: a PR binds by a fact — its head commit in the worktree's
//! reflog, the ticket's own `msmn/` branch as its head, or its URL in the
//! result of a `pr create` a session of the ticket ran — and only among the
//! PRs the signed-in account authored, so a URL a script prints or a
//! commit an agent fetched from a stranger's PR binds nothing.

use serde_json::Value;

/// Which forge a repository lives on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Forge {
    GitHub,
    GitLab,
}

impl Forge {
    /// The wire and feed word.
    pub fn word(self) -> &'static str {
        match self {
            Forge::GitHub => "github",
            Forge::GitLab => "gitlab",
        }
    }

    pub fn from_word(w: &str) -> Option<Forge> {
        match w {
            "github" => Some(Forge::GitHub),
            "gitlab" => Some(Forge::GitLab),
            _ => None,
        }
    }

    /// The forge's own name, for a sentence: `GitHub`, `GitLab`.
    pub fn name(self) -> &'static str {
        match self {
            Forge::GitHub => "GitHub",
            Forge::GitLab => "GitLab",
        }
    }

    /// What the forge calls one: `PR` or `MR`.
    pub fn noun(self) -> &'static str {
        match self {
            Forge::GitHub => "PR",
            Forge::GitLab => "MR",
        }
    }

    /// The forge's own spelling of a number: `#42` or `!7`. The sign alone
    /// says which forge, so the card needs no word.
    pub fn label(self, number: u64) -> String {
        match self {
            Forge::GitHub => format!("#{number}"),
            Forge::GitLab => format!("!{number}"),
        }
    }
}

/// A PR's state, as the board reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrState {
    Open,
    Draft,
    Merged,
    /// Closed without merging: the team set the work aside. It stays in a
    /// ticket's set (T-797), so the board still knows the work went through
    /// a pull request, and judges no pipeline of it.
    Closed,
}

impl PrState {
    pub fn word(self) -> &'static str {
        match self {
            PrState::Open => "open",
            PrState::Draft => "draft",
            PrState::Merged => "merged",
            PrState::Closed => "closed",
        }
    }

    pub fn from_word(w: &str) -> Option<PrState> {
        match w {
            "open" => Some(PrState::Open),
            "draft" => Some(PrState::Draft),
            "merged" => Some(PrState::Merged),
            "closed" => Some(PrState::Closed),
            _ => None,
        }
    }

    pub fn is_open(self) -> bool {
        matches!(self, PrState::Open | PrState::Draft)
    }
}

/// The pipeline of a commit — GitHub's checks and GitLab's pipelines under
/// one word. Ordered by how loud it is: a failure outranks a wait, a wait a
/// run, a run a cancelled one, and that a pass, so the worst of several is
/// the fold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pipeline {
    /// Not asked yet, or the forge did not answer: never read as green.
    Unknown,
    /// The forge ran nothing for this commit.
    None,
    Passed,
    /// Ended with no verdict (T-800): cancelled — GitLab's auto-cancel of a
    /// redundant pipeline, a workflow's `cancel-in-progress` — or GitHub's
    /// `stale`. Quiet, never green: nothing failed, and nothing passed.
    Cancelled(String),
    /// The word names the job the forge says is running, where it said
    /// (T-794): a queued one where none has started yet.
    Running(String),
    /// Stopped on a person: a manual job, an environment waiting for its
    /// reviewer. The word names the job where the forge said.
    Waiting(String),
    /// The word names the failing job where the forge said.
    Failed(String),
}

impl Pipeline {
    pub fn word(&self) -> &'static str {
        match self {
            Pipeline::Unknown => "unknown",
            Pipeline::None => "none",
            Pipeline::Passed => "passed",
            Pipeline::Cancelled(_) => "cancelled",
            Pipeline::Running(_) => "running",
            Pipeline::Waiting(_) => "waiting",
            Pipeline::Failed(_) => "failed",
        }
    }

    /// The job the word is about, where there is one.
    pub fn job(&self) -> Option<&str> {
        match self {
            Pipeline::Cancelled(j)
            | Pipeline::Running(j)
            | Pipeline::Waiting(j)
            | Pipeline::Failed(j)
                if !j.is_empty() =>
            {
                Some(j)
            }
            _ => None,
        }
    }

    pub fn from_words(word: &str, job: &str) -> Pipeline {
        let job = job.to_string();
        match word {
            "none" => Pipeline::None,
            "passed" => Pipeline::Passed,
            "cancelled" => Pipeline::Cancelled(job),
            "running" => Pipeline::Running(job),
            "waiting" => Pipeline::Waiting(job),
            "failed" => Pipeline::Failed(job),
            _ => Pipeline::Unknown,
        }
    }

    fn rank(&self) -> u8 {
        match self {
            Pipeline::Unknown => 0,
            Pipeline::None => 1,
            Pipeline::Passed => 2,
            Pipeline::Cancelled(_) => 3,
            Pipeline::Running(_) => 4,
            Pipeline::Waiting(_) => 5,
            Pipeline::Failed(_) => 6,
        }
    }

    /// The louder of two.
    pub fn worst(self, other: Pipeline) -> Pipeline {
        if other.rank() > self.rank() {
            other
        } else {
            self
        }
    }
}

/// A reviewer's verdict on an open PR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Review {
    #[default]
    None,
    Required,
    Approved,
    ChangesRequested,
}

impl Review {
    pub fn word(self) -> &'static str {
        match self {
            Review::None => "none",
            Review::Required => "required",
            Review::Approved => "approved",
            Review::ChangesRequested => "changes_requested",
        }
    }

    pub fn from_word(w: &str) -> Review {
        match w {
            "required" => Review::Required,
            "approved" => Review::Approved,
            "changes_requested" => Review::ChangesRequested,
            _ => Review::None,
        }
    }
}

/// One pull request as both CLIs' JSON reads into it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pr {
    pub forge: Forge,
    /// `owner/repo` or `group/sub/project`, as the remote spells it.
    pub repo: String,
    pub number: u64,
    pub url: String,
    pub state: PrState,
    /// The head commit, what `made here` matches against the reflog.
    pub head_oid: String,
    pub head_ref: String,
    /// The commit the merge landed as, once merged: a squash's, a merge
    /// commit's, or the head itself under a fast-forward.
    pub merge_oid: String,
    pub conflict: bool,
    pub review: Review,
    pub pipeline: Pipeline,
    /// Unix ms the forge last touched it; what orders a ticket's set.
    pub updated_ms: u64,
    /// Unix ms it merged, zero where it has not or the forge did not say
    /// (T-800): what "the newest merged" means, since `updated_ms` moves on
    /// any comment or bot post.
    pub merged_ms: u64,
    /// The environments the merge commit reached, as the forge names them;
    /// read only where a column asks for one, and only once merged.
    pub deployed: Vec<String>,
    /// The ticket's branch tips this merged PR's head carries though it is
    /// not one of them (T-799): a reviewer's commit on the forge put the
    /// head past the worktree's tip, and the merge landed the tip all the
    /// same. Read by ancestry on the daemon's worker (`merged_past`), never
    /// off the forge's list.
    pub landed_tips: Vec<String>,
}

impl Pr {
    pub fn label(&self) -> String {
        self.forge.label(self.number)
    }

    /// When it merged, for ordering merged PRs (T-800): the forge's merge
    /// time, else its last touch where the forge gave none (a record from
    /// an older build).
    pub fn merged_order(&self) -> u64 {
        if self.merged_ms > 0 {
            self.merged_ms
        } else {
            self.updated_ms
        }
    }

    /// Whether this PR's head is `tip` or carries it (T-799).
    pub fn lands(&self, tip: &str) -> bool {
        !tip.is_empty() && (self.head_oid == tip || self.landed_tips.iter().any(|t| t == tip))
    }
}

/// Each repository's newest merged PR in a set, by when it merged (T-800),
/// never by `updated_ms`, which a comment on an older PR moves. In the
/// order the set first names each repository.
pub fn newest_merged(set: &[Pr]) -> Vec<&Pr> {
    let mut out: Vec<&Pr> = Vec::new();
    for p in set.iter().filter(|p| p.state == PrState::Merged) {
        match out.iter_mut().find(|n| n.repo == p.repo) {
            Some(n) => {
                if (p.merged_order(), p.number) > (n.merged_order(), n.number) {
                    *n = p;
                }
            }
            None => out.push(p),
        }
    }
    out
}

/// Read `gh pr list --json …`'s array, or `gh pr view --json …`'s one
/// object (T-800). A line that is not that shape reads as no PRs, never as
/// an error: a forge's words are untrusted text and a surprise in them must
/// not take the sample down.
pub fn parse_gh(repo: &str, json: &str) -> Vec<Pr> {
    let items = match serde_json::from_str::<Value>(json) {
        Ok(Value::Array(items)) => items,
        Ok(one @ Value::Object(_)) => vec![one],
        _ => return Vec::new(),
    };
    items
        .iter()
        .filter_map(|v| {
            let number = v.get("number")?.as_u64()?;
            let state = match v.get("state").and_then(Value::as_str)? {
                "MERGED" => PrState::Merged,
                "CLOSED" => PrState::Closed,
                _ if v.get("isDraft").and_then(Value::as_bool) == Some(true) => PrState::Draft,
                _ => PrState::Open,
            };
            let merge_oid = v
                .get("mergeCommit")
                .and_then(|m| m.get("oid"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let review = match v.get("reviewDecision").and_then(Value::as_str) {
                Some("APPROVED") => Review::Approved,
                Some("CHANGES_REQUESTED") => Review::ChangesRequested,
                Some("REVIEW_REQUIRED") => Review::Required,
                _ => Review::None,
            };
            Some(Pr {
                forge: Forge::GitHub,
                repo: repo.to_string(),
                number,
                url: web_url(v, "url"),
                state,
                head_oid: str_of(v, "headRefOid"),
                head_ref: str_of(v, "headRefName"),
                merge_oid,
                conflict: v.get("mergeable").and_then(Value::as_str) == Some("CONFLICTING"),
                review,
                pipeline: gh_rollup(v.get("statusCheckRollup")),
                updated_ms: iso_ms(v.get("updatedAt")),
                merged_ms: iso_ms(v.get("mergedAt")),
                deployed: Vec::new(),
                landed_tips: Vec::new(),
            })
        })
        .collect()
}

/// The one `pipeline` word for a PR's `statusCheckRollup`: the worst of its
/// check runs and status contexts, a failing one named, and a running one
/// named by the first check in progress, else the first queued (T-794).
pub fn gh_rollup(rollup: Option<&Value>) -> Pipeline {
    let Some(Value::Array(entries)) = rollup else { return Pipeline::None };
    let mut fold = Pipeline::None;
    // Whether the running word's job is one in progress, not one queued.
    let mut started = false;
    for e in entries {
        let name = job_word(
            e.get("name").or_else(|| e.get("context")).and_then(Value::as_str).unwrap_or_default(),
        );
        // GraphQL (`gh pr list`) spells these in capitals, REST
        // (`commits/{sha}/check-runs`) in lower case: one reading.
        let word =
            |key: &str| e.get(key).and_then(Value::as_str).unwrap_or_default().to_ascii_uppercase();
        let (status, conclusion, state) = (word("status"), word("conclusion"), word("state"));
        // A commit status (`StatusContext`) says `state` alone.
        let one = match (status.as_str(), conclusion.as_str(), state.as_str()) {
            (_, "FAILURE" | "TIMED_OUT" | "STARTUP_FAILURE", _) | (_, _, "FAILURE" | "ERROR") => {
                Pipeline::Failed(name)
            }
            (_, "ACTION_REQUIRED", _) | ("WAITING", _, _) => Pipeline::Waiting(name),
            (_, "SUCCESS" | "NEUTRAL" | "SKIPPED", _) | (_, _, "SUCCESS") => Pipeline::Passed,
            // Cancelled, GitHub's `stale` (a run left incomplete for two
            // weeks), or a verdict this build does not know: ended, and
            // never a pass (T-800).
            ("COMPLETED", _, _) | (_, "CANCELLED" | "STALE", _) => Pipeline::Cancelled(name),
            ("IN_PROGRESS", _, _) if !started && matches!(fold, Pipeline::Running(_)) => {
                started = true;
                fold = Pipeline::Running(name);
                continue;
            }
            ("IN_PROGRESS", _, _) => {
                started = true;
                Pipeline::Running(name)
            }
            // Queued, pending, requested, expected.
            _ => Pipeline::Running(name),
        };
        fold = fold.worst(one);
    }
    fold
}

/// Read `glab mr list -F json`'s array. The list carries no pipeline: that
/// is read by sha afterwards (`parse_gitlab_pipelines`) and stays `Unknown`
/// here.
pub fn parse_glab(repo: &str, json: &str) -> Vec<Pr> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(json) else { return Vec::new() };
    items.iter().filter_map(|v| glab_mr(repo, v)).collect()
}

/// GitLab's read of one MR (`projects/:id/merge_requests/:iid`, T-784): the
/// MR as the list spells it, and its head pipeline's word and id where it
/// names one for that head. GitLab's list serves a head minutes old after a
/// push; this read does not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MrRead {
    pub pr: Pr,
    pub pipeline: Option<(Pipeline, u64)>,
}

pub fn parse_glab_mr(repo: &str, json: &str) -> Option<MrRead> {
    let v = serde_json::from_str::<Value>(json).ok()?;
    let pr = glab_mr(repo, &v)?;
    let pipeline = v.get("head_pipeline").filter(|h| h.is_object()).and_then(|h| {
        // A pipeline of another commit says nothing of this head.
        let sha = h.get("sha").and_then(Value::as_str).unwrap_or_default();
        if !sha.is_empty() && sha != pr.head_oid {
            return None;
        }
        let word = gitlab_status(h.get("status").and_then(Value::as_str).unwrap_or_default());
        (word != Pipeline::Unknown).then_some((word, h.get("id")?.as_u64()?))
    });
    Some(MrRead { pr, pipeline })
}

impl Pr {
    /// What a single read says of an open MR over its list row (T-784,
    /// T-787): the state, the head, the conflict, the merge status's review
    /// word and the commit a merge landed as. The list lags both a push and
    /// a merge. Its time where it is the later one, so a set ordered by it
    /// still puts the MR the forge touched last first. A read of another MR
    /// changes nothing.
    pub fn take_read(&mut self, read: &Pr) {
        if read.repo != self.repo || read.number != self.number || read.head_oid.is_empty() {
            return;
        }
        self.state = read.state;
        self.head_oid = read.head_oid.clone();
        self.conflict = read.conflict;
        self.review = read.review;
        self.merge_oid = read.merge_oid.clone();
        self.updated_ms = self.updated_ms.max(read.updated_ms);
        self.merged_ms = read.merged_ms;
    }
}

/// One MR object, as the list and the single read both spell it.
fn glab_mr(repo: &str, v: &Value) -> Option<Pr> {
    let number = v.get("iid")?.as_u64()?;
    let state = match v.get("state").and_then(Value::as_str)? {
        "merged" => PrState::Merged,
        "closed" => PrState::Closed,
        _ if v.get("draft").and_then(Value::as_bool) == Some(true) => PrState::Draft,
        _ => PrState::Open,
    };
    // What is on the target branch (T-754). Under GitLab's default
    // "merge commit" method a squash still lands as a merge commit
    // whose second parent is the squash commit, and the MR carries
    // both shas: the pipeline and the deployments are the merge
    // commit's. A fast-forward or semi-linear squash has no merge
    // commit, and the squash sha is then what landed.
    // A fast-forward merge without a squash has neither (T-795): the head
    // itself is what landed.
    let merged_keys: &[&str] = if state == PrState::Merged {
        &["merge_commit_sha", "squash_commit_sha", "sha"]
    } else {
        &["merge_commit_sha", "squash_commit_sha"]
    };
    let merge_oid = merged_keys
        .iter()
        .filter_map(|k| v.get(*k).and_then(Value::as_str))
        .find(|s| !s.is_empty())
        .unwrap_or_default()
        .to_string();
    let status = v.get("detailed_merge_status").and_then(Value::as_str).unwrap_or_default();
    let conflict =
        v.get("has_conflicts").and_then(Value::as_bool) == Some(true) || status == "conflict";
    let review = match status {
        "not_approved" => Review::Required,
        "requested_changes" => Review::ChangesRequested,
        _ => Review::None,
    };
    Some(Pr {
        forge: Forge::GitLab,
        repo: repo.to_string(),
        number,
        url: web_url(v, "web_url"),
        state,
        head_oid: str_of(v, "sha"),
        head_ref: str_of(v, "source_branch"),
        merge_oid,
        conflict,
        review,
        pipeline: Pipeline::Unknown,
        updated_ms: iso_ms(v.get("updated_at")),
        merged_ms: iso_ms(v.get("merged_at")),
        deployed: Vec::new(),
        landed_tips: Vec::new(),
    })
}

/// The one `pipeline` word for GitLab's `pipelines?sha=` answer: the newest
/// pipeline's status, and its id for the read of its jobs (T-772). GitLab
/// names no job on this road: a wait or a failure reads bare until the
/// pipeline's jobs are read (`parse_gitlab_job`).
pub fn parse_gitlab_pipelines(json: &str) -> (Pipeline, Option<u64>) {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(json) else {
        return (Pipeline::Unknown, None);
    };
    let Some(first) = items.first() else { return (Pipeline::None, None) };
    let status = gitlab_status(first.get("status").and_then(Value::as_str).unwrap_or_default());
    (status, first.get("id").and_then(Value::as_u64))
}

/// The job a GitLab pipeline stopped on, out of its `pipelines/:id/jobs`
/// answer (T-772): the first job at `status` — `manual` for a waiting
/// pipeline, `failed` for a failed one, `running` for one in flight
/// (T-794) — a job allowed to fail only where
/// no other did. Empty where none is.
pub fn parse_gitlab_job(json: &str, status: &str) -> String {
    let Ok(Value::Array(jobs)) = serde_json::from_str::<Value>(json) else { return String::new() };
    let at: Vec<&Value> =
        jobs.iter().filter(|j| j.get("status").and_then(Value::as_str) == Some(status)).collect();
    let allowed = |j: &&&Value| j.get("allow_failure").and_then(Value::as_bool) == Some(true);
    at.iter()
        .find(|j| !allowed(j))
        .or_else(|| at.first())
        .and_then(|j| j.get("name").and_then(Value::as_str))
        .map(job_word)
        .unwrap_or_default()
}

impl Pipeline {
    /// The GitLab job status that names this word's job: `running` for a
    /// run (T-794), `manual` for a wait, `failed` for a failure, `None` for
    /// a word with no job or one already named. A cancelled pipeline's jobs
    /// are all cancelled with it, and none is the one (T-800).
    pub fn gitlab_job_status(&self) -> Option<&'static str> {
        match self {
            Pipeline::Running(j) if j.is_empty() => Some("running"),
            Pipeline::Waiting(j) if j.is_empty() => Some("manual"),
            Pipeline::Failed(j) if j.is_empty() => Some("failed"),
            _ => None,
        }
    }

    /// The same word, about `job`.
    pub fn with_job(self, job: String) -> Pipeline {
        match self {
            Pipeline::Running(_) => Pipeline::Running(job),
            Pipeline::Waiting(_) => Pipeline::Waiting(job),
            Pipeline::Failed(_) => Pipeline::Failed(job),
            Pipeline::Cancelled(_) => Pipeline::Cancelled(job),
            other => other,
        }
    }
}

/// A GitLab pipeline status word as the board's.
pub fn gitlab_status(status: &str) -> Pipeline {
    match status {
        "success" => Pipeline::Passed,
        "failed" => Pipeline::Failed(String::new()),
        // Auto-cancelled as redundant, or by a person (T-800).
        "canceled" | "cancelled" | "canceling" => Pipeline::Cancelled(String::new()),
        "manual" => Pipeline::Waiting(String::new()),
        "skipped" => Pipeline::None,
        "created" | "waiting_for_resource" | "preparing" | "pending" | "running" | "scheduled" => {
            Pipeline::Running(String::new())
        }
        _ => Pipeline::Unknown,
    }
}

/// The one `pipeline` word for GitHub's `commits/{sha}/check-runs` answer
/// (`{"check_runs": [...]}`): the merge commit's pipeline after the merge.
pub fn parse_gh_check_runs(json: &str) -> Pipeline {
    let Ok(v) = serde_json::from_str::<Value>(json) else { return Pipeline::Unknown };
    match v.get("check_runs") {
        Some(Value::Array(a)) if a.is_empty() => Pipeline::None,
        Some(runs @ Value::Array(_)) => gh_rollup(Some(runs)),
        _ => Pipeline::Unknown,
    }
}

/// The one `pipeline` word for GitHub's `commits/{sha}/status` answer
/// (`{"state": …, "statuses": [...]}`): the commit statuses a CI that
/// predates check runs reports (Jenkins, older CircleCI), folded as the
/// rollup folds a `StatusContext` (T-800).
pub fn parse_gh_commit_status(json: &str) -> Pipeline {
    let Ok(v) = serde_json::from_str::<Value>(json) else { return Pipeline::Unknown };
    match v.get("statuses") {
        Some(Value::Array(a)) if a.is_empty() => Pipeline::None,
        Some(statuses @ Value::Array(_)) => gh_rollup(Some(statuses)),
        _ => Pipeline::Unknown,
    }
}

/// A GitHub merge commit's pipeline: the worse of its check runs and its
/// commit statuses (T-800), since a repository's CI may report either. One
/// read that did not answer leaves the whole unknown: the other's pass is
/// not the commit's.
pub fn gh_commit_pipeline(check_runs: Pipeline, statuses: Pipeline) -> Pipeline {
    if check_runs == Pipeline::Unknown || statuses == Pipeline::Unknown {
        return Pipeline::Unknown;
    }
    check_runs.worst(statuses)
}

/// A string field, scrubbed: the forge's words are untrusted text and this
/// is the boundary they cross (`text::scrub_text`).
fn str_of(v: &Value, key: &str) -> String {
    crate::text::scrub_text(v.get(key).and_then(Value::as_str).unwrap_or_default())
}

/// A PR's page, kept only as an `http(s)://` address (T-795): the board
/// hands it to the opener, and a forge's answer is no more trusted than a
/// note's link, which `links` admits on the same rule. Empty otherwise.
fn web_url(v: &Value, key: &str) -> String {
    let url = str_of(v, key);
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("https://") || lower.starts_with("http://") {
        url
    } else {
        String::new()
    }
}

/// How many bytes of a job's or an environment's name the board keeps: a
/// name is a word on a card and a clause in the crown's wake, never a
/// paragraph a CI config could write there.
pub const NAME_MAX_BYTES: usize = 80;

/// A job's or an environment's name as the board keeps it: scrubbed, one
/// line, capped at `NAME_MAX_BYTES`.
pub fn job_word(raw: &str) -> String {
    // One line first — a control character is a space here, not nothing —
    // then the scrub, then the cap.
    let one: String = raw.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let flat = crate::text::scrub_text(&one).split_whitespace().collect::<Vec<_>>().join(" ");
    crate::text::cap_bytes(&flat, NAME_MAX_BYTES).to_string()
}

/// GitHub's `deployments?sha=` answer: each deployment's id and environment.
pub fn parse_gh_deployments(json: &str) -> Vec<(u64, String)> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(json) else { return Vec::new() };
    items
        .iter()
        .filter_map(|d| {
            let id = d.get("id")?.as_u64()?;
            let env = d.get("environment").and_then(Value::as_str)?;
            Some((id, job_word(env)))
        })
        .collect()
}

/// GitHub's `deployments/{id}/statuses` answer, newest first: when the
/// deployment turned success, unix ms (zero where it names no time), or
/// `None` where it never did. Any success counts, not only the newest
/// status (T-800): GitHub's `auto_inactive` marks a successful deployment
/// `inactive` once a newer one succeeds, and it was deployed all the same.
pub fn parse_gh_deployment_success(json: &str) -> Option<u64> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(json) else { return None };
    let success =
        items.iter().find(|s| s.get("state").and_then(Value::as_str) == Some("success"))?;
    Some(iso_ms(success.get("created_at")))
}

/// GitLab's `deployments?environment=&status=success` answer: when one of
/// them deployed this commit, unix ms (zero where it names no time), or
/// `None` where none did.
pub fn parse_gitlab_deployed(json: &str, oid: &str) -> Option<u64> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(json) else { return None };
    if oid.is_empty() {
        return None;
    }
    let d = items.iter().find(|d| {
        d.get("sha").and_then(Value::as_str) == Some(oid)
            || d.get("deployable")
                .and_then(|x| x.get("commit"))
                .and_then(|c| c.get("id"))
                .and_then(Value::as_str)
                == Some(oid)
    })?;
    Some(
        ["finished_at", "updated_at", "created_at"]
            .iter()
            .map(|k| iso_ms(d.get(*k)))
            .find(|ms| *ms > 0)
            .unwrap_or(0),
    )
}

/// The environments one read found a commit deployed to, in the order they
/// happened (T-784): by the deployment's time where the forge gave every
/// one, else by `envs`, the board's column order — never by name.
pub fn deploy_order(mut found: Vec<(String, u64)>, envs: &[String]) -> Vec<String> {
    let rank = |e: &str| envs.iter().position(|x| x == e).unwrap_or(usize::MAX);
    if found.iter().all(|(_, at)| *at > 0) {
        found.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| rank(&a.0).cmp(&rank(&b.0))));
    } else {
        found.sort_by_key(|(e, _)| rank(e));
    }
    found.into_iter().map(|(e, _)| e).collect()
}

/// An ISO-8601 stamp (`2026-10-13T05:56:17Z`, or with an offset) as unix
/// ms, to the whole second (`usage::rfc3339_ms`); zero where it is not one.
/// Whole seconds are what both forges send.
pub fn iso_ms(v: Option<&Value>) -> u64 {
    v.and_then(Value::as_str).and_then(crate::usage::rfc3339_ms).map_or(0, |ms| ms - ms % 1000)
}

/// A git remote, parsed: the host and the repository path the forge CLI
/// names it by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remote {
    pub host: String,
    /// `owner/repo`, or `group/sub/project`; never a leading slash or a
    /// `.git`.
    pub path: String,
}

impl Remote {
    /// `https://github.com/o/r.git`, `git@github.com:o/r.git`,
    /// `ssh://git@gitlab.example.com:2222/g/s/p.git`, `https://gitlab.com/g/p`.
    pub fn parse(url: &str) -> Option<Remote> {
        let url = url.trim();
        let (host, path) =
            if let Some((scheme_less, _)) = url.split_once("://").map(|(_, r)| (r, ())) {
                let (hostport, path) = scheme_less.split_once('/')?;
                let host = hostport.rsplit('@').next()?.split(':').next()?;
                (host, path)
            } else if let Some((userhost, path)) = url.split_once(':') {
                // scp-like: `git@host:path`
                if path.starts_with("//") {
                    return None;
                }
                let host = userhost.rsplit('@').next()?;
                (host, path)
            } else {
                return None;
            };
        let path = path.trim_matches('/').trim_end_matches(".git").trim_matches('/');
        if host.is_empty() || path.is_empty() || !path.contains('/') {
            return None;
        }
        Some(Remote { host: canonical_host(host), path: path.to_string() })
    }

    /// The forge the host is known to be, before any CLI is asked:
    /// github.com is GitHub; any other host is asked of both CLIs.
    /// `host/path`: the repository's one name across the board's reads.
    pub fn key(&self) -> String {
        format!("{}/{}", self.host, self.path)
    }

    pub fn known_forge(&self) -> Option<Forge> {
        (self.host == "github.com").then_some(Forge::GitHub)
    }

    /// The forges whose CLI is asked of the host, in order: GitHub's alone
    /// for github.com, any other host GitLab's and then GitHub's.
    pub fn candidates(&self) -> &'static [Forge] {
        match self.known_forge() {
            Some(Forge::GitHub) => &[Forge::GitHub],
            _ => &[Forge::GitLab, Forge::GitHub],
        }
    }
}

/// A host as the forge CLIs name it: lower case, and a forge's SSH-over-443
/// host as the forge itself (T-800) — `ssh.github.com` is github.com,
/// `altssh.gitlab.com` gitlab.com.
pub fn canonical_host(host: &str) -> String {
    let host = host.to_ascii_lowercase();
    match host.as_str() {
        "ssh.github.com" => "github.com".into(),
        "altssh.gitlab.com" => "gitlab.com".into(),
        _ => host,
    }
}

/// The SSH host a remote URL reaches its forge through, where git would
/// speak SSH: an scp-like `git@github-work:o/r.git` or an `ssh://` URL
/// (T-800). The name may be an `~/.ssh/config` alias, which `ssh -G`
/// resolves. `None` for any other URL, and for a name that is not a plain
/// host name, so nothing a remote spells reaches `ssh` as an option.
pub fn ssh_alias(url: &str) -> Option<String> {
    let url = url.trim();
    let hostpart = match url.split_once("://") {
        Some((scheme, rest)) => {
            if !matches!(scheme.to_ascii_lowercase().as_str(), "ssh" | "git+ssh" | "ssh+git") {
                return None;
            }
            let authority = rest.split('/').next()?;
            let host = authority.rsplit('@').next()?;
            // `[::1]:22` is no alias; `host:2222` is the host.
            host.split(':').next()?
        }
        None => {
            let (userhost, path) = url.split_once(':')?;
            if path.starts_with("//") || userhost.contains('/') {
                return None;
            }
            userhost.rsplit('@').next()?
        }
    };
    let plain = !hostpart.is_empty()
        && !hostpart.starts_with('-')
        && hostpart.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    plain.then(|| hostpart.to_string())
}

/// The `hostname` an `ssh -G <alias>` answer resolves to, where it says one.
pub fn ssh_config_hostname(out: &str) -> Option<String> {
    out.lines().find_map(|l| {
        let (key, value) = l.trim().split_once(' ')?;
        let value = value.trim();
        (key.eq_ignore_ascii_case("hostname") && !value.is_empty()).then(|| value.to_string())
    })
}

/// The facts a ticket offers for binding: what its worktree legs hold and
/// what its sessions printed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facts {
    /// The ticket's own `msmn/` branch.
    pub branch: String,
    /// Every commit a worktree leg's HEAD ever pointed at (its reflog),
    /// beside every path a PR of the leg's repository may name, as the
    /// remotes spell them: a fork's own and its upstream's (T-800).
    pub reflog: Vec<(Vec<String>, Vec<String>)>,
    /// URLs a `pr create` the ticket's sessions ran printed.
    pub opened: Vec<String>,
}

/// Does this PR belong to the ticket? A fact, never a mention: its head is
/// a commit made in the ticket's worktree for that repository, its head is
/// the ticket's own branch, or a session of the ticket opened it and the
/// forge printed this URL back.
pub fn binds(pr: &Pr, facts: &Facts) -> bool {
    if facts.opened.iter().any(|u| same_url(u, &pr.url)) {
        return true;
    }
    if !facts.branch.is_empty() && pr.head_ref == facts.branch {
        return true;
    }
    !pr.head_oid.is_empty()
        && facts
            .reflog
            .iter()
            .any(|(repos, oids)| repos.contains(&pr.repo) && oids.contains(&pr.head_oid))
}

/// Two URLs naming the same PR, a trailing slash or `.diff` aside.
pub fn same_url(a: &str, b: &str) -> bool {
    let norm = |u: &str| {
        u.trim()
            .trim_end_matches('/')
            .trim_end_matches(".diff")
            .trim_end_matches(".patch")
            .to_string()
    };
    norm(a) == norm(b)
}

/// Whether a URL a session printed names this PR (T-797): the same URL, or
/// its number and repository path, the path in any case, as both forges
/// route it. A remote spelled `Owner/Repo` names the PR the CLI printed as
/// `owner/repo`.
pub fn url_names(url: &str, repo: &str, number: u64) -> bool {
    pr_number_of(url).is_some_and(|(_, n)| n == number)
        && repo_of_url(url).is_some_and(|r| r.path.eq_ignore_ascii_case(repo))
}

/// A ticket's set out of the bound PRs, newest first, a closed one included
/// (T-797): the merge stays the forge's for a ticket whose PRs are all
/// closed, as for one whose PRs merged (T-778). A closed PR's pipeline,
/// conflict and review were about work the team set aside, and read as
/// none, so no fold, edge or mark reads them.
pub fn ticket_set(mut prs: Vec<Pr>) -> Vec<Pr> {
    for p in prs.iter_mut().filter(|p| p.state == PrState::Closed) {
        p.pipeline = Pipeline::None;
        p.conflict = false;
        p.review = Review::None;
    }
    prs.sort_by(|a, b| b.updated_ms.cmp(&a.updated_ms).then(b.number.cmp(&a.number)));
    prs.dedup_by(|a, b| a.repo == b.repo && a.number == b.number);
    prs
}

/// Whether a set holds a PR with a pipeline to judge: an open or a merged
/// one. A set of closed PRs alone has none (T-797), and the pipeline gate
/// reads it as no PR bound.
pub fn pipelined(set: &[Pr]) -> bool {
    set.iter().any(|p| p.state != PrState::Closed)
}

/// The set's merged PR for one repository at one tip: the newest merged PR
/// there whose head is that tip, or carries it (`Pr::lands`, T-799). A
/// commit after the merge is work that has not landed, as the git road
/// reads it, since no merged head is or carries the tip then. A later PR
/// from another branch, merged or open, leaves the landing the one that
/// names it (T-786): the lab's !11 from `fix/a` merged after !9 had, and
/// the leg's words fell back to `origin/main`. An open one holds the
/// ticket's merge on its own (`holding`, T-774). `repos` is every path
/// the repository's PRs may name (T-800): a PR opened from a fork against
/// its upstream names the upstream's.
pub fn merged_at<'a>(set: &'a [Pr], repos: &[String], tip: &str) -> Option<&'a Pr> {
    set.iter().find(|p| repos.contains(&p.repo) && p.state == PrState::Merged && p.lands(tip))
}

/// A merged PR whose head may carry the leg's tip without being it
/// (T-799), which only ancestry says: one from the ticket's own branch, its
/// head and the tip both known and different. A reviewer's commit on the
/// forge puts the head past the tip, and a squash of it is the ticket's
/// merge all the same. A PR from another branch lands only at its head: its
/// commits are not the ticket's branch, and a head that carries the tip
/// there is a later branch cut from it.
pub fn merged_past(pr: &Pr, branch: &str, tip: &str) -> bool {
    pr.state == PrState::Merged
        && !branch.is_empty()
        && pr.head_ref == branch
        && is_oid(&pr.head_oid)
        && is_oid(tip)
        && pr.head_oid != tip
}

/// A full commit id: 40 or 64 hex digits. A forge's head is untrusted text
/// and reaches git's argv, where anything else could read as an option.
pub fn is_oid(s: &str) -> bool {
    matches!(s.len(), 40 | 64) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The commit ids of a PR's commits page (T-799): GitHub's
/// `pulls/:n/commits` (`sha`) and GitLab's merge request `commits` (`id`).
/// `None` for an answer that is not a list, so an error page is never read
/// as "the tip is not there".
pub fn parse_pr_commits(json: &str) -> Option<Vec<String>> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(json) else { return None };
    Some(
        items
            .iter()
            .filter_map(|c| c.get("sha").or_else(|| c.get("id")).and_then(Value::as_str))
            .map(str::to_string)
            .collect(),
    )
}

/// The PR a ticket's merge waits on (T-774): the newest open one, on any
/// repository and any branch. While one is open the ticket's work has not
/// landed, whatever its branch reads: a follow-up MR after the branch merged
/// holds the merge gate as it holds the train.
pub fn holding(set: &[Pr]) -> Option<&Pr> {
    set.iter().find(|p| p.state.is_open())
}

/// Why the merge is the forge's (T-739), in the one sentence `m`, the crown,
/// the train and the merge gate say: `MR !2 open — merge it on GitLab`.
pub fn open_refusal(pr: &Pr) -> String {
    format!("{} {} open — merge it on {}", pr.forge.noun(), pr.label(), pr.forge.name())
}

/// Why a ticket's branch is not landed here (T-778), or `None`: the open
/// PR's sentence while one is open (`open_refusal`), else, with a PR
/// merged, that the forge carries the merge. Its PRs merged at heads other
/// than the branch's tip leave the branch reading ahead of a local base the
/// forge never moved, and a fast-forward there lands the work twice. With
/// every PR closed unmerged (T-797) the team set the work aside on the
/// forge, and reopening the newest is the road back to a merge.
pub fn merge_refusal(set: &[Pr]) -> Option<String> {
    if let Some(open) = holding(set) {
        return Some(open_refusal(open));
    }
    if let Some(merged) = set.iter().find(|p| p.state == PrState::Merged) {
        return Some(format!(
            "its pull requests carry the merge — merge on {}",
            merged.forge.name()
        ));
    }
    let closed = set.first()?;
    Some(format!(
        "{} {} closed unmerged — reopen it on {}",
        closed.forge.noun(),
        closed.label(),
        closed.forge.name()
    ))
}

/// Why the forge is who merges a ticket with a PR bound (T-778), in
/// `get_ticket`'s `merge.why`, or `None` with none bound: its PRs closed
/// unmerged, every one (T-797), else its work goes through them.
pub fn merge_why(set: &[Pr]) -> Option<&'static str> {
    if set.is_empty() {
        None
    } else if pipelined(set) {
        Some("its work goes through pull requests")
    } else {
        Some("its pull requests were closed unmerged on the forge")
    }
}

/// The worst pipeline over the set's open PRs; the merged commit's once
/// nothing is open and one is merged (the caller has read it onto the PR);
/// `Unknown` with no PR. An open PR whose pipeline was not read makes a
/// fold that would read green or none `Unknown` (T-795): unknown is never
/// green, and a run, a wait or a failure beside it still says itself.
pub fn pipeline_of(set: &[Pr]) -> Pipeline {
    let open: Vec<&Pr> = set.iter().filter(|p| p.state.is_open()).collect();
    if !open.is_empty() {
        let worst = open.iter().fold(Pipeline::None, |acc, p| acc.worst(p.pipeline.clone()));
        let unread = open.iter().any(|p| p.pipeline == Pipeline::Unknown);
        return match worst {
            Pipeline::None | Pipeline::Passed if unread => Pipeline::Unknown,
            worst => worst,
        };
    }
    match set.iter().find(|p| p.state == PrState::Merged) {
        Some(p) => p.pipeline.clone(),
        None => Pipeline::Unknown,
    }
}

/// Why a column that needs a passed pipeline refuses a ticket (T-739), or
/// `None` when it may enter. `set` is the forge's last reading of the
/// ticket's PRs; `unread` whether its transcripts opened one a forge read
/// still owes, `None` when they could not be read. A ticket with no PR
/// bound and none unread has no pipeline to pass and enters, as a ticket
/// with no branch passes the merge rule (T-750); one opened here and not
/// yet read by the forge waits for it, so a turn's end seconds after a `pr
/// create` is not waved through. A set of closed PRs alone is none bound
/// (T-797): a closed PR has no pipeline to pass. Unknown is never green.
pub fn entry_refusal(set: Option<&[Pr]>, unread: Option<bool>) -> Option<String> {
    let set = set.filter(|s| pipelined(s));
    if set.is_none() && unread == Some(false) {
        return None;
    }
    match set.map(pipeline_of).unwrap_or(Pipeline::Unknown) {
        Pipeline::Passed => None,
        Pipeline::Unknown => Some("pipeline not read".into()),
        Pipeline::None => Some("no pipeline ran".into()),
        Pipeline::Running(_) => Some("pipeline running".into()),
        Pipeline::Waiting(job) if job.is_empty() => Some("pipeline waiting".into()),
        Pipeline::Waiting(job) => Some(format!("pipeline waiting: {job}")),
        Pipeline::Failed(job) if job.is_empty() => Some("pipeline failed".into()),
        Pipeline::Failed(job) => Some(format!("pipeline failed: {job}")),
        Pipeline::Cancelled(job) if job.is_empty() => Some("pipeline cancelled".into()),
        Pipeline::Cancelled(job) => Some(format!("pipeline cancelled: {job}")),
    }
}

/// An open PR from the ticket's branch whose head is not its leg's tip now
/// (T-780): the forge read the commit before a push, or the tip was never
/// pushed, so its pipeline says nothing of the branch as it stands and
/// reads as not read. `tips` is each leg's repository, as a `Pr`'s `repo`
/// spells it, with its branch's tip. A PR from another branch, one on a
/// repository with no leg, and an empty head or tip are not judged.
pub fn head_unread(set: &[Pr], branch: &str, tips: &[(String, String)]) -> bool {
    set.iter()
        .filter(|p| p.state.is_open() && !p.head_oid.is_empty())
        .filter(|p| p.head_ref.is_empty() || p.head_ref == branch)
        .any(|p| {
            tips.iter().any(|(repo, tip)| *repo == p.repo && !tip.is_empty() && *tip != p.head_oid)
        })
}

/// The transcript detector (the `opened here` fact). A pass keeps the ids
/// of `pr create` tool calls whose result has not come yet in `pending`,
/// and returns the PR URLs a result printed. Claude's JSONL (`tool_use` /
/// `tool_result` blocks) and Codex's rollout (`function_call` /
/// `function_call_output`) both read; anything else is a line that says
/// nothing.
pub fn pr_create_line(line: &str, pending: &mut Vec<String>) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<Value>(line) else { return Vec::new() };
    let mut out = Vec::new();
    // Claude: the message's content blocks.
    if let Some(Value::Array(blocks)) = v.get("message").and_then(|m| m.get("content")) {
        for b in blocks {
            match b.get("type").and_then(Value::as_str) {
                Some("tool_use") => {
                    let command = b
                        .get("input")
                        .and_then(|i| i.get("command"))
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if is_pr_create(command) {
                        if let Some(id) = b.get("id").and_then(Value::as_str) {
                            remember(pending, id);
                        }
                    }
                }
                Some("tool_result") => {
                    let Some(id) = b.get("tool_use_id").and_then(Value::as_str) else { continue };
                    if let Some(at) = pending.iter().position(|p| p == id) {
                        pending.remove(at);
                        out.extend(pr_urls(&flatten(b.get("content"))));
                    }
                }
                _ => {}
            }
        }
    }
    // Codex: `response_item` payloads.
    if let Some(p) = v.get("payload") {
        match p.get("type").and_then(Value::as_str) {
            Some("function_call") => {
                let args = p.get("arguments").and_then(Value::as_str).unwrap_or_default();
                if is_pr_create(args) {
                    if let Some(id) = p.get("call_id").and_then(Value::as_str) {
                        remember(pending, id);
                    }
                }
            }
            Some("function_call_output") => {
                if let Some(id) = p.get("call_id").and_then(Value::as_str) {
                    if let Some(at) = pending.iter().position(|q| q == id) {
                        pending.remove(at);
                        out.extend(pr_urls(&flatten(p.get("output"))));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// How many unanswered `pr create` ids a cursor keeps: a result that never
/// comes (an interrupted turn) must not grow the file.
const PENDING_CAP: usize = 8;

fn remember(pending: &mut Vec<String>, id: &str) {
    if pending.iter().any(|p| p == id) {
        return;
    }
    if pending.len() >= PENDING_CAP {
        pending.remove(0);
    }
    pending.push(id.to_string());
}

/// A shell command that opens a PR: `gh pr create …` or `glab mr create …`,
/// anywhere in the line (after a `cd`, under `&&`), never `pr view` or a
/// mention in prose, since this is the tool's own input.
pub fn is_pr_create(command: &str) -> bool {
    let words: Vec<&str> = command.split_whitespace().collect();
    words.windows(3).any(|w| {
        (w[0].ends_with("gh") && w[1] == "pr" && w[2] == "create")
            || (w[0].ends_with("glab") && w[1] == "mr" && w[2] == "create")
    })
}

/// A tool result's text: a string, or the text blocks of an array.
fn flatten(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Every PR URL in a text: `https://<host>/<path>/pull/<n>` or
/// `…/-/merge_requests/<n>`, as the two CLIs print them.
pub fn pr_urls(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in
        text.split(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '<' || c == '>')
    {
        let Some(at) = word.find("https://") else { continue };
        // A JSON-encoded tool result spells its newline `\n` (T-795).
        let url = word[at..].split('\\').next().unwrap_or_default();
        let url = url.trim_end_matches(['.', ',', ';', ':', ')', ']']);
        if pr_number_of(url).is_some() && !out.iter().any(|u| same_url(u, url)) {
            out.push(url.to_string());
        }
    }
    out
}

/// The repository a PR URL names: its host and path, the PR's own segments
/// cut off.
pub fn repo_of_url(url: &str) -> Option<Remote> {
    let (forge, _) = pr_number_of(url)?;
    let cut = match forge {
        Forge::GitHub => "/pull/",
        Forge::GitLab => "/-/merge_requests/",
    };
    let head = url.split_once("://")?.1;
    let (repo, _) = head.rsplit_once(cut)?;
    // GitLab's `-/` is cut with the segment; a path that spelled
    // `/merge_requests/` without it is read the same way.
    let repo = repo.trim_end_matches("/-");
    Remote::parse(&format!("https://{repo}"))
}

/// The PR number a URL names, and which forge's shape it has.
pub fn pr_number_of(url: &str) -> Option<(Forge, u64)> {
    let path = url.split_once("://")?.1;
    let segs: Vec<&str> = path.split('/').collect();
    let n = segs.len();
    if n >= 4 && segs[n - 2] == "pull" {
        return segs[n - 1].parse().ok().map(|k| (Forge::GitHub, k));
    }
    if n >= 4 && segs[n - 2] == "merge_requests" {
        return segs[n - 1].parse().ok().map(|k| (Forge::GitLab, k));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pr(repo: &str, number: u64, state: PrState, head: &str, updated: u64) -> Pr {
        Pr {
            forge: Forge::GitHub,
            repo: repo.into(),
            number,
            url: format!("https://github.com/{repo}/pull/{number}"),
            state,
            head_oid: head.into(),
            head_ref: "fix/x".into(),
            merge_oid: String::new(),
            conflict: false,
            review: Review::None,
            pipeline: Pipeline::None,
            updated_ms: updated,
            merged_ms: 0,
            deployed: Vec::new(),
            landed_tips: Vec::new(),
        }
    }

    #[test]
    fn gh_json_reads_into_a_pr_with_its_pipeline_folded() {
        let json = r#"[{"number":14651,"url":"https://github.com/cli/cli/pull/14651","state":"OPEN","isDraft":false,
          "headRefName":"fix/issue-9789","headRefOid":"5d74d92f","mergeCommit":null,"mergedAt":null,
          "mergeable":"CONFLICTING","reviewDecision":"CHANGES_REQUESTED","updatedAt":"2026-10-13T05:56:17Z",
          "statusCheckRollup":[
            {"__typename":"CheckRun","name":"lint","status":"COMPLETED","conclusion":"SUCCESS"},
            {"__typename":"CheckRun","name":"test-linux","status":"COMPLETED","conclusion":"FAILURE"},
            {"__typename":"CheckRun","name":"build","status":"IN_PROGRESS","conclusion":null},
            {"__typename":"StatusContext","context":"ci/circle","state":"SUCCESS"}]},
          {"number":14650,"url":"https://github.com/cli/cli/pull/14650","state":"MERGED","isDraft":false,
          "headRefName":"b","headRefOid":"94c5a6fa","mergeCommit":{"oid":"abc123"},"mergedAt":"2026-10-12T23:12:40Z",
          "mergeable":"UNKNOWN","reviewDecision":"APPROVED","updatedAt":"2026-10-12T23:12:40Z","statusCheckRollup":[]},
          {"number":1,"url":"u","state":"OPEN","isDraft":true,"headRefName":"d","headRefOid":"x","updatedAt":"bad"}]"#;
        let prs = parse_gh("cli/cli", json);
        assert_eq!(prs.len(), 3);
        assert_eq!(prs[0].state, PrState::Open);
        assert!(prs[0].conflict);
        assert_eq!(prs[0].review, Review::ChangesRequested);
        assert_eq!(
            prs[0].pipeline,
            Pipeline::Failed("test-linux".into()),
            "a failure outranks a run"
        );
        assert_eq!(prs[0].updated_ms, 1_791_870_977_000);
        assert_eq!(prs[1].state, PrState::Merged);
        assert_eq!(prs[1].merge_oid, "abc123");
        assert_eq!(prs[1].pipeline, Pipeline::None, "an empty rollup ran nothing");
        assert_eq!(prs[2].state, PrState::Draft);
        assert_eq!(prs[2].updated_ms, 0);
        assert_eq!(prs[0].label(), "#14651");
        assert!(parse_gh("x", "not json").is_empty());
        assert!(parse_gh("x", "{}").is_empty());
        let one = r#"{"number":7,"url":"https://github.com/o/r/pull/7","state":"MERGED","headRefName":"b",
          "headRefOid":"h","mergeCommit":{"oid":"m"},"mergedAt":"2026-10-12T23:12:40Z"}"#;
        let view = parse_gh("o/r", one);
        assert_eq!(view.len(), 1, "`pr view`'s one object");
        assert_eq!((view[0].number, view[0].state), (7, PrState::Merged));
    }

    #[test]
    fn the_rollup_folds_to_the_loudest_word() {
        let v = |s: &str| serde_json::from_str::<Value>(s).unwrap();
        let running = v(
            r#"[{"name":"a","status":"QUEUED"},{"name":"b","status":"COMPLETED","conclusion":"SUCCESS"}]"#,
        );
        assert_eq!(gh_rollup(Some(&running)), Pipeline::Running("a".into()));
        let started = v(
            r#"[{"name":"lint","status":"QUEUED"},{"name":"build","status":"IN_PROGRESS"},{"name":"test","status":"IN_PROGRESS"}]"#,
        );
        assert_eq!(
            gh_rollup(Some(&started)),
            Pipeline::Running("build".into()),
            "the first check in progress names a running pipeline over a queued one"
        );
        let waiting =
            v(r#"[{"name":"deploy-prod","status":"WAITING"},{"name":"b","status":"IN_PROGRESS"}]"#);
        assert_eq!(gh_rollup(Some(&waiting)), Pipeline::Waiting("deploy-prod".into()));
        let required =
            v(r#"[{"name":"deploy","status":"COMPLETED","conclusion":"ACTION_REQUIRED"}]"#);
        assert_eq!(gh_rollup(Some(&required)), Pipeline::Waiting("deploy".into()));
        let passed = v(
            r#"[{"name":"a","status":"COMPLETED","conclusion":"SKIPPED"},{"context":"x","state":"SUCCESS"}]"#,
        );
        assert_eq!(gh_rollup(Some(&passed)), Pipeline::Passed);
        assert_eq!(gh_rollup(None), Pipeline::None);
        assert_eq!(parse_gh_check_runs(r#"{"check_runs":[]}"#), Pipeline::None);
        assert_eq!(
            parse_gh_check_runs(
                r#"{"check_runs":[{"name":"release","status":"completed","conclusion":"FAILURE"}]}"#
            ),
            Pipeline::Failed("release".into())
        );
        assert_eq!(parse_gh_check_runs("nope"), Pipeline::Unknown);
        assert_eq!(
            parse_gh_check_runs(
                r#"{"check_runs":[{"name":"release","status":"completed","conclusion":"success"}]}"#
            ),
            Pipeline::Passed,
            "REST spells its words in lower case"
        );
    }

    /// T-800: the newest merged PR of a repository is the one that merged
    /// last, whatever was commented on since; both CLIs' merge time reads.
    #[test]
    fn the_newest_merged_pr_is_the_last_to_merge_not_the_last_touched() {
        let gh = r#"[{"number":5,"url":"https://github.com/o/r/pull/5","state":"MERGED","headRefName":"a","headRefOid":"h5",
          "mergeCommit":{"oid":"m5"},"mergedAt":"2026-10-13T06:00:00Z","updatedAt":"2026-10-13T06:00:00Z"},
          {"number":4,"url":"https://github.com/o/r/pull/4","state":"MERGED","headRefName":"b","headRefOid":"h4",
          "mergeCommit":{"oid":"m4"},"mergedAt":"2026-10-12T06:00:00Z","updatedAt":"2026-10-14T06:00:00Z"}]"#;
        let set = ticket_set(parse_gh("o/r", gh));
        assert_eq!(set[0].number, 4, "a bot's comment on #4 moved it first in the set");
        assert_eq!(set[1].merged_ms, 1_791_871_200_000);
        let newest: Vec<u64> = newest_merged(&set).iter().map(|p| p.number).collect();
        assert_eq!(newest, [5], "but #5 merged last");
        let glab = r#"[{"iid":3,"web_url":"https://gitlab.com/g/p/-/merge_requests/3","state":"merged","source_branch":"x",
          "sha":"s","merge_commit_sha":"m","merged_at":"2026-10-13T04:00:00Z","updated_at":"2026-10-14T04:00:00Z"}]"#;
        assert_eq!(parse_glab("g/p", glab)[0].merged_ms, 1_791_864_000_000);
        let mut old = set[0].clone();
        old.merged_ms = 0;
        assert_eq!(old.merged_order(), old.updated_ms, "an older record's time stands in");
        let mut other = set[1].clone();
        other.repo = "o/s".into();
        let two = [set[0].clone(), set[1].clone(), other];
        let newest: Vec<(&str, u64)> =
            newest_merged(&two).iter().map(|p| (p.repo.as_str(), p.number)).collect();
        assert_eq!(newest, [("o/r", 5), ("o/s", 5)], "one per repository");
    }

    /// T-800: a cancelled run is its own quiet word, never green and never
    /// a failure; GitHub's `stale` is no pass; and a merge commit's commit
    /// statuses count beside its check runs.
    #[test]
    fn a_cancelled_or_stale_run_is_neither_green_nor_failed() {
        let v = |s: &str| serde_json::from_str::<Value>(s).unwrap();
        let cancelled = v(
            r#"[{"name":"deploy","status":"COMPLETED","conclusion":"CANCELLED"},{"name":"lint","status":"COMPLETED","conclusion":"SUCCESS"}]"#,
        );
        assert_eq!(gh_rollup(Some(&cancelled)), Pipeline::Cancelled("deploy".into()));
        let stale = v(r#"[{"name":"e2e","status":"COMPLETED","conclusion":"STALE"}]"#);
        assert_eq!(gh_rollup(Some(&stale)), Pipeline::Cancelled("e2e".into()), "stale is no pass");
        let unknown = v(r#"[{"name":"x","status":"COMPLETED","conclusion":"SOMETHING_NEW"}]"#);
        assert_ne!(gh_rollup(Some(&unknown)), Pipeline::Passed, "an unknown verdict is no pass");
        let neutral = v(r#"[{"name":"x","status":"COMPLETED","conclusion":"NEUTRAL"}]"#);
        assert_eq!(gh_rollup(Some(&neutral)), Pipeline::Passed);
        let mixed = v(
            r#"[{"name":"a","status":"COMPLETED","conclusion":"CANCELLED"},{"name":"b","status":"IN_PROGRESS"}]"#,
        );
        assert_eq!(gh_rollup(Some(&mixed)), Pipeline::Running("b".into()), "a run is louder");
        let failed = v(
            r#"[{"name":"a","status":"COMPLETED","conclusion":"CANCELLED"},{"name":"b","status":"COMPLETED","conclusion":"FAILURE"}]"#,
        );
        assert_eq!(gh_rollup(Some(&failed)), Pipeline::Failed("b".into()));
        assert_eq!(
            parse_gh_check_runs(
                r#"{"check_runs":[{"name":"deploy","status":"completed","conclusion":"cancelled"}]}"#
            ),
            Pipeline::Cancelled("deploy".into())
        );
        let c = Pipeline::Cancelled(String::new());
        assert_eq!(Pipeline::from_words(c.word(), "deploy"), Pipeline::Cancelled("deploy".into()));
        assert_eq!(Pipeline::Cancelled("deploy".into()).job(), Some("deploy"));
        assert_eq!(c.gitlab_job_status(), None, "every job of a cancelled pipeline is cancelled");
        let merged = |p: Pipeline| {
            let mut m = pr("o/r", 1, PrState::Merged, "h", 1);
            m.pipeline = p;
            vec![m]
        };
        assert_eq!(
            entry_refusal(Some(&merged(Pipeline::Cancelled(String::new()))), None).as_deref(),
            Some("pipeline cancelled")
        );
        assert_eq!(
            entry_refusal(Some(&merged(Pipeline::Cancelled("deploy".into()))), None).as_deref(),
            Some("pipeline cancelled: deploy")
        );

        // Commit statuses: Jenkins reports these, not check runs.
        let jenkins = r#"{"state":"success","statuses":[{"context":"continuous-integration/jenkins","state":"success"}],"total_count":1}"#;
        assert_eq!(parse_gh_commit_status(jenkins), Pipeline::Passed);
        let pending = r#"{"state":"pending","statuses":[{"context":"ci/circleci: build","state":"pending"}]}"#;
        assert_eq!(parse_gh_commit_status(pending), Pipeline::Running("ci/circleci: build".into()));
        let errored = r#"{"state":"failure","statuses":[{"context":"jenkins","state":"error"}]}"#;
        assert_eq!(parse_gh_commit_status(errored), Pipeline::Failed("jenkins".into()));
        assert_eq!(parse_gh_commit_status(r#"{"state":"pending","statuses":[]}"#), Pipeline::None);
        assert_eq!(parse_gh_commit_status("{}"), Pipeline::Unknown);
        let none = parse_gh_check_runs(r#"{"check_runs":[]}"#);
        assert_eq!(
            gh_commit_pipeline(none.clone(), parse_gh_commit_status(jenkins)),
            Pipeline::Passed,
            "a commit whose CI reports statuses alone has a pipeline"
        );
        assert_eq!(gh_commit_pipeline(none.clone(), Pipeline::None), Pipeline::None);
        assert_eq!(
            gh_commit_pipeline(Pipeline::Passed, parse_gh_commit_status(errored)),
            Pipeline::Failed("jenkins".into()),
            "the worse of the two"
        );
        assert_eq!(gh_commit_pipeline(Pipeline::Passed, Pipeline::Unknown), Pipeline::Unknown);
        assert_eq!(gh_commit_pipeline(Pipeline::Unknown, Pipeline::Passed), Pipeline::Unknown);
    }

    #[test]
    fn glab_json_reads_into_a_pr_and_its_pipeline_comes_by_sha() {
        let json = r#"[{"iid":4096,"web_url":"https://gitlab.com/gitlab-org/cli/-/merge_requests/4096","state":"opened",
          "source_branch":"fix/x","sha":"deadbeef","merge_commit_sha":null,"squash_commit_sha":null,"draft":false,
          "has_conflicts":true,"detailed_merge_status":"conflict","updated_at":"2026-10-13T06:00:00.000+02:00"},
          {"iid":7,"web_url":"https://gitlab.com/g/p/-/merge_requests/7","state":"merged","source_branch":"y","sha":"s",
          "merge_commit_sha":"m1","squash_commit_sha":"sq1","updated_at":"2026-10-13T04:00:00Z"}]"#;
        let prs = parse_glab("gitlab-org/cli", json);
        assert_eq!(prs.len(), 2);
        assert!(prs[0].conflict);
        assert_eq!(prs[0].pipeline, Pipeline::Unknown, "the list says nothing of the pipeline");
        assert_eq!(prs[0].label(), "!4096");
        assert_eq!(prs[0].updated_ms, 1_791_864_000_000, "+02:00 is two hours earlier in UTC");
        assert_eq!(
            prs[1].merge_oid, "m1",
            "the merge commit before the squash commit: under GitLab's default merge method \
             a squash lands as a merge commit, and that is what is on main (T-754)"
        );
        let ff = r#"[{"iid":8,"web_url":"https://gitlab.com/g/p/-/merge_requests/8","state":"merged","source_branch":"z","sha":"s",
          "merge_commit_sha":null,"squash_commit_sha":"sq2","updated_at":"2026-10-13T04:00:00Z"}]"#;
        assert_eq!(
            parse_glab("g/p", ff)[0].merge_oid,
            "sq2",
            "a fast-forward or semi-linear squash has no merge commit: the squash sha landed"
        );
        // A fast-forward merge without a squash has neither: the head landed
        // (T-795). An open MR's empty shas stay empty.
        let plain_ff = r#"[{"iid":9,"web_url":"https://gitlab.com/g/p/-/merge_requests/9","state":"merged","source_branch":"z","sha":"h9",
          "merge_commit_sha":null,"squash_commit_sha":null,"detailed_merge_status":"requested_changes","updated_at":"2026-10-13T04:00:00Z"},
          {"iid":10,"web_url":"javascript:alert(1)","state":"opened","source_branch":"w","sha":"h10",
          "merge_commit_sha":null,"squash_commit_sha":null,"updated_at":"2026-10-13T04:00:00Z"}]"#;
        let read = parse_glab("g/p", plain_ff);
        assert_eq!(read[0].merge_oid, "h9");
        assert_eq!(read[0].review, Review::ChangesRequested, "GitLab's request for changes");
        assert_eq!(read[1].merge_oid, "");
        assert_eq!(read[1].url, "", "only a web page is handed to the opener");
        assert_eq!(gitlab_status("canceling"), Pipeline::Cancelled(String::new()));
        assert_eq!(gitlab_status("canceled"), Pipeline::Cancelled(String::new()));
        assert_eq!(
            parse_gitlab_pipelines(r#"[{"status":"manual"}]"#).0,
            Pipeline::Waiting(String::new())
        );
        assert_eq!(
            parse_gitlab_pipelines(r#"[{"status":"success"},{"status":"failed"}]"#).0,
            Pipeline::Passed
        );
        assert_eq!(parse_gitlab_pipelines("[]"), (Pipeline::None, None));
        assert_eq!(gitlab_status("running"), Pipeline::Running(String::new()));
        assert_eq!(gitlab_status("failed"), Pipeline::Failed(String::new()));
    }

    /// T-784, lab run 3: five minutes after a push GitLab's list still
    /// served !9 at the failed commit; the single read named the pushed
    /// head and its passed pipeline. The read's head, conflict and pipeline
    /// win over the list row.
    #[test]
    fn a_single_mr_read_overrides_the_lists_stale_head() {
        let list = r#"[{"iid":9,"web_url":"https://gitlab.com/o/api/-/merge_requests/9","state":"opened",
          "source_branch":"msmn/T-13-x","sha":"dc82695","draft":false,"has_conflicts":true,
          "detailed_merge_status":"conflict","updated_at":"2026-10-14T21:31:22Z"}]"#;
        let mut pr = parse_glab("o/api", list).remove(0);
        assert_eq!(pr.head_oid, "dc82695");
        let one = r#"{"iid":9,"web_url":"https://gitlab.com/o/api/-/merge_requests/9","state":"opened",
          "source_branch":"msmn/T-13-x","sha":"ebf3084","draft":false,"has_conflicts":false,
          "detailed_merge_status":"mergeable","updated_at":"2026-10-14T21:34:00Z",
          "head_pipeline":{"id":2101,"sha":"ebf3084","status":"success"}}"#;
        let read = parse_glab_mr("o/api", one).unwrap();
        assert_eq!(read.pipeline, Some((Pipeline::Passed, 2101)));
        pr.take_read(&read.pr);
        assert_eq!(pr.head_oid, "ebf3084", "the single read's head");
        assert!(!pr.conflict, "and its conflict");
        assert_eq!(pr.updated_ms, read.pr.updated_ms, "and its later time");
        let mut stale = read.pr.clone();
        stale.updated_ms = 0;
        pr.take_read(&stale);
        assert_eq!(pr.updated_ms, read.pr.updated_ms, "never an earlier one");
        assert_eq!(pr.url, parse_glab("o/api", list)[0].url, "the rest is the list's");

        // A head pipeline of another commit, or none, leaves the pipeline to
        // the read by sha; a read of another MR changes nothing.
        let other = one.replace(r#""sha":"ebf3084","status""#, r#""sha":"dc82695","status""#);
        assert_eq!(parse_glab_mr("o/api", &other).unwrap().pipeline, None);
        let bare = one.replace(
            r#""head_pipeline":{"id":2101,"sha":"ebf3084","status":"success"}"#,
            r#""head_pipeline":null"#,
        );
        assert_eq!(parse_glab_mr("o/api", &bare).unwrap().pipeline, None);
        let failed = one.replace("success", "failed");
        assert_eq!(
            parse_glab_mr("o/api", &failed).unwrap().pipeline,
            Some((Pipeline::Failed(String::new()), 2101))
        );
        let mut eight = parse_glab("o/api", &list.replace(r#""iid":9"#, r#""iid":8"#)).remove(0);
        eight.take_read(&read.pr);
        assert_eq!(eight.head_oid, "dc82695");
        assert_eq!(parse_glab_mr("o/api", "[]"), None);
    }

    /// T-787, lab run 3: !11 merged on GitLab at 00:59:27, and a minute
    /// later the list still served it `opened` at a current head, so
    /// REVIEW's `pipeline passes` took the ticket back from PROD. The single
    /// read's state wins, and with it the commit the merge landed as.
    #[test]
    fn a_single_mr_read_overrides_the_lists_stale_state() {
        let list = r#"[{"iid":11,"web_url":"https://gitlab.com/o/api/-/merge_requests/11","state":"opened",
          "source_branch":"fix/a","sha":"a11","draft":false,"has_conflicts":false,
          "detailed_merge_status":"mergeable","updated_at":"2026-10-14T21:58:00Z"}]"#;
        let mut pr = parse_glab("o/api", list).remove(0);
        assert_eq!(pr.state, PrState::Open);
        let one = r#"{"iid":11,"web_url":"https://gitlab.com/o/api/-/merge_requests/11","state":"merged",
          "source_branch":"fix/a","sha":"a11","draft":false,"has_conflicts":false,
          "detailed_merge_status":"not_open","updated_at":"2026-10-14T21:59:27Z",
          "merge_commit_sha":"m11","squash_commit_sha":null,
          "head_pipeline":{"id":2201,"sha":"a11","status":"success"}}"#;
        pr.take_read(&parse_glab_mr("o/api", one).unwrap().pr);
        assert_eq!(pr.state, PrState::Merged, "the single read's state");
        assert_eq!(pr.merge_oid, "m11", "and the commit it landed as");
        assert_eq!(holding(&[pr.clone()]), None, "no open MR holds the ticket");
        let draft = one
            .replace(r#""state":"merged""#, r#""state":"opened""#)
            .replace(r#""draft":false"#, r#""draft":true"#);
        pr.take_read(&parse_glab_mr("o/api", &draft).unwrap().pr);
        assert_eq!(pr.state, PrState::Draft, "a read says draft as the list does");
    }

    /// T-784, C2: staging deployed 5 s before production, and the wake said
    /// production first. The forge's times order the words; with a time
    /// missing the board's column order does, never the names.
    #[test]
    fn deployments_are_said_in_the_order_they_happened() {
        let envs = vec!["staging".to_string(), "production".to_string()];
        let found = vec![("production".to_string(), 2_000), ("staging".to_string(), 1_000)];
        assert_eq!(deploy_order(found, &envs), ["staging", "production"]);
        let late = vec![("staging".to_string(), 9_000), ("production".to_string(), 2_000)];
        assert_eq!(deploy_order(late, &envs), ["production", "staging"], "time beats the columns");
        let untimed = vec![("production".to_string(), 2_000), ("staging".to_string(), 0)];
        assert_eq!(deploy_order(untimed, &envs), ["staging", "production"], "the columns' order");
    }

    #[test]
    fn a_gitlab_pipeline_names_the_job_it_stopped_on() {
        assert_eq!(
            parse_gitlab_pipelines(r#"[{"id":44,"status":"manual"},{"id":43}]"#),
            (Pipeline::Waiting(String::new()), Some(44))
        );
        let jobs = r#"[{"name":"test","status":"success"},
            {"name":"lint","status":"failed","allow_failure":true},
            {"name":"deploy:production","status":"manual"},
            {"name":"build\nhere","status":"failed","allow_failure":false}]"#;
        assert_eq!(parse_gitlab_job(jobs, "manual"), "deploy:production");
        assert_eq!(
            parse_gitlab_job(jobs, "failed"),
            "build here",
            "a job allowed to fail is named only where no other failed, through job_word"
        );
        assert_eq!(parse_gitlab_job(r#"[{"name":"lint","status":"failed"}]"#, "manual"), "");
        assert_eq!(parse_gitlab_job("not json", "failed"), "");
        let waiting = Pipeline::Waiting(String::new());
        assert_eq!(waiting.gitlab_job_status(), Some("manual"));
        assert_eq!(Pipeline::Failed(String::new()).gitlab_job_status(), Some("failed"));
        assert_eq!(Pipeline::Failed("x".into()).gitlab_job_status(), None, "named already");
        assert_eq!(Pipeline::Running(String::new()).gitlab_job_status(), Some("running"));
        assert_eq!(Pipeline::Running("build".into()).gitlab_job_status(), None);
        assert_eq!(
            Pipeline::Running(String::new()).with_job("build".into()),
            Pipeline::Running("build".into())
        );
        assert_eq!(
            parse_gitlab_job(
                r#"[{"name":"lint","status":"success"},{"name":"test linux","status":"running"}]"#,
                "running"
            ),
            "test linux"
        );
        assert_eq!(waiting.with_job("deploy".into()), Pipeline::Waiting("deploy".into()));
    }

    #[test]
    fn a_remote_url_names_its_host_and_path() {
        let r = Remote::parse("https://github.com/amitozalvo/mesimon.git").unwrap();
        assert_eq!((r.host.as_str(), r.path.as_str()), ("github.com", "amitozalvo/mesimon"));
        assert_eq!(r.known_forge(), Some(Forge::GitHub));
        let r = Remote::parse("git@github.com:o/r.git").unwrap();
        assert_eq!((r.host.as_str(), r.path.as_str()), ("github.com", "o/r"));
        let r = Remote::parse("ssh://git@gitlab.example.com:2222/g/s/p.git").unwrap();
        assert_eq!((r.host.as_str(), r.path.as_str()), ("gitlab.example.com", "g/s/p"));
        assert_eq!(r.known_forge(), None, "a host that is not github.com is asked");
        let r = Remote::parse("https://user:tok@GitLab.com/g/p/").unwrap();
        assert_eq!((r.host.as_str(), r.path.as_str()), ("gitlab.com", "g/p"));
        assert_eq!(Remote::parse("/local/path/repo.git"), None);
        assert_eq!(Remote::parse("https://github.com/"), None);
        assert_eq!(Remote::parse("https://host/onlyone"), None);
    }

    /// T-800: GitHub's SSH-over-443 host is github.com, and an SSH remote's
    /// host is the name `ssh -G` resolves, never one that could be an option.
    #[test]
    fn an_ssh_remote_names_the_host_ssh_would_reach() {
        let r = Remote::parse("ssh://git@ssh.github.com:443/o/r.git").unwrap();
        assert_eq!(r.key(), "github.com/o/r");
        assert_eq!(r.known_forge(), Some(Forge::GitHub));
        assert_eq!(Remote::parse("git@altssh.gitlab.com:g/p.git").unwrap().host, "gitlab.com");
        assert_eq!(ssh_alias("git@github-work:o/r.git").as_deref(), Some("github-work"));
        assert_eq!(ssh_alias("github-work:o/r.git").as_deref(), Some("github-work"));
        assert_eq!(ssh_alias("ssh://git@gh_work:2222/o/r.git").as_deref(), Some("gh_work"));
        assert_eq!(ssh_alias("git+ssh://git@gh.work/o/r").as_deref(), Some("gh.work"));
        assert_eq!(ssh_alias("https://github.com/o/r.git"), None, "not SSH");
        assert_eq!(ssh_alias("/srv/git/r.git"), None);
        assert_eq!(ssh_alias("./a:b"), None, "a path with a colon");
        assert_eq!(ssh_alias("git@-oProxyCommand=x:o/r"), None, "never an option");
        assert_eq!(ssh_alias("git@a b:o/r"), None);
        let out = "user git\nhostname github.com\nport 22\n";
        assert_eq!(ssh_config_hostname(out).as_deref(), Some("github.com"));
        assert_eq!(ssh_config_hostname("port 22\n"), None);
    }

    #[test]
    fn a_pr_binds_by_a_fact_and_never_by_a_mention() {
        let facts = Facts {
            branch: "msmn/T-12-x".into(),
            reflog: vec![(vec!["o/r".into()], vec!["aaa".into(), "bbb".into()])],
            opened: vec!["https://gitlab.com/g/p/-/merge_requests/7/".into()],
        };
        assert!(binds(&pr("o/r", 1, PrState::Open, "bbb", 1), &facts), "made here");
        assert!(
            !binds(&pr("o/other", 1, PrState::Open, "bbb", 1), &facts),
            "another repo's commit"
        );
        let mut on_branch = pr("o/r", 2, PrState::Open, "zzz", 1);
        on_branch.head_ref = "msmn/T-12-x".into();
        assert!(binds(&on_branch, &facts), "the ticket's own branch");
        let mut opened = pr("g/p", 7, PrState::Open, "qqq", 1);
        opened.url = "https://gitlab.com/g/p/-/merge_requests/7".into();
        assert!(binds(&opened, &facts), "opened here, a trailing slash aside");
        assert!(!binds(&pr("o/r", 9, PrState::Open, "nope", 1), &facts), "a mention binds nothing");
    }

    #[test]
    fn the_set_is_newest_first_closed_included() {
        let mut closed = pr("o/r", 2, PrState::Closed, "b", 20);
        closed.pipeline = Pipeline::Failed("test".into());
        closed.conflict = true;
        closed.review = Review::ChangesRequested;
        let set = ticket_set(vec![
            pr("o/r", 1, PrState::Merged, "a", 10),
            closed,
            pr("o/r", 3, PrState::Open, "c", 30),
            pr("o/r", 3, PrState::Open, "c", 30),
        ]);
        assert_eq!(set.iter().map(|p| p.number).collect::<Vec<_>>(), [3, 2, 1]);
        // T-797: a closed PR stays, its pipeline, conflict and review read
        // as none, and it lands nothing.
        let closed = &set[1];
        assert_eq!(
            (closed.pipeline.clone(), closed.conflict, closed.review),
            (Pipeline::None, false, Review::None)
        );
        assert!(pipelined(&set));
        assert!(!pipelined(&set[1..2]), "a closed PR alone has no pipeline");
        assert_eq!(merged_at(&set, &["o/r".into()], "b"), None);
        assert_eq!(holding(&set[1..2]), None);
        assert_eq!(merged_at(&set, &["o/r".into()], "c"), None, "the newest is open");
        let landed = ticket_set(vec![pr("o/r", 1, PrState::Merged, "a", 10)]);
        assert_eq!(merged_at(&landed, &["o/r".into()], "a").map(|p| p.number), Some(1));
        assert_eq!(
            merged_at(&landed, &["o/r".into()], "a2"),
            None,
            "a commit after the merge is unlanded"
        );
        assert_eq!(merged_at(&landed, &["o/r".into()], ""), None);
        // A later PR from another branch keeps the landing named (T-786),
        // merged or open, and on another repository it is not looked at.
        let mut later = pr("o/r", 11, PrState::Merged, "f", 60);
        later.head_ref = "fix/a".into();
        let both = ticket_set(vec![pr("o/r", 1, PrState::Merged, "a", 10), later.clone()]);
        assert_eq!(merged_at(&both, &["o/r".into()], "a").map(|p| p.number), Some(1));
        assert_eq!(merged_at(&both, &["o/r".into()], "f").map(|p| p.number), Some(11));
        later.state = PrState::Open;
        let open = ticket_set(vec![pr("o/r", 1, PrState::Merged, "a", 10), later]);
        assert_eq!(merged_at(&open, &["o/r".into()], "a").map(|p| p.number), Some(1));
        assert_eq!(merged_at(&open, &["o/x".into()], "a"), None);
        // A fork's leg (T-800): `origin` is me/r, the PR was opened against
        // the upstream o/r, and either path names the repository.
        let fork: Vec<String> = vec!["me/r".into(), "o/r".into()];
        assert_eq!(merged_at(&landed, &fork, "a").map(|p| p.number), Some(1));
        let facts = Facts { reflog: vec![(fork.clone(), vec!["a".into()])], ..Default::default() };
        assert!(binds(&landed[0], &facts), "a commit made in the fork binds the upstream's PR");
        let mine =
            Facts { reflog: vec![(vec!["me/r".into()], vec!["a".into()])], ..Default::default() };
        assert!(!binds(&landed[0], &mine), "and only a path the leg's remotes name");
        let mut running = pr("o/r", 4, PrState::Open, "d", 40);
        running.pipeline = Pipeline::Running(String::new());
        let mut failed = pr("o/r", 5, PrState::Draft, "e", 50);
        failed.pipeline = Pipeline::Failed("t".into());
        assert_eq!(pipeline_of(&[failed.clone(), running.clone()]), Pipeline::Failed("t".into()));
        let mut merged = pr("o/r", 6, PrState::Merged, "f", 60);
        merged.pipeline = Pipeline::Passed;
        assert_eq!(
            pipeline_of(&[merged]),
            Pipeline::Passed,
            "the merge commit's once nothing is open"
        );
        assert_eq!(pipeline_of(&[]), Pipeline::Unknown);
    }

    /// T-799: a reviewer's commit on the forge put #42's head one past the
    /// worktree's tip, and the squash landed. The head carries the tip, so
    /// the tip is merged; a commit after it is not, and nor is a tip the
    /// head does not carry.
    #[test]
    fn a_merged_head_past_the_tip_lands_the_tip_it_carries() {
        let (tip, head, after) = ("a".repeat(40), "b".repeat(40), "c".repeat(40));
        let mut p = pr("o/r", 42, PrState::Merged, &head, 10);
        p.head_ref = "msmn/T-9-x".into();
        assert!(merged_past(&p, "msmn/T-9-x", &tip), "its own branch, its head past the tip");
        let (here, there) = (["o/r".to_string()], ["o/x".to_string()]);
        assert_eq!(merged_at(std::slice::from_ref(&p), &here, &tip), None, "not read yet");
        p.landed_tips.push(tip.clone());
        assert!(p.lands(&tip) && p.lands(&head) && !p.lands(&after) && !p.lands(""));
        let set = [p.clone()];
        assert_eq!(merged_at(&set, &here, &tip).map(|p| p.number), Some(42));
        assert_eq!(merged_at(&set, &here, &after), None, "a commit after the merge");
        assert_eq!(merged_at(&set, &there, &tip), None, "another repository");

        // Only a merged PR of the ticket's own branch whose head is not the
        // tip is asked, and only on real commit ids.
        assert!(!merged_past(&p, "msmn/T-9-x", &head), "the head is the tip");
        assert!(!merged_past(&p, "msmn/T-1-y", &tip), "another ticket's branch");
        assert!(!merged_past(&p, "", &tip));
        assert!(!merged_past(&p, "msmn/T-9-x", "--output=/tmp/x"), "never an option");
        let mut open = p.clone();
        open.state = PrState::Open;
        assert!(!merged_past(&open, "msmn/T-9-x", &tip), "an open PR is T-780's");
        let mut side = p.clone();
        side.head_ref = "fix/a".into();
        assert!(!merged_past(&side, "msmn/T-9-x", &tip), "a side branch lands at its head");
        assert!(is_oid(&"0f".repeat(32)) && !is_oid("abc") && !is_oid(&"g".repeat(40)));
    }

    #[test]
    fn a_prs_commits_read_off_either_forge_and_an_error_page_is_none() {
        let gh = r#"[{"sha":"aaa","commit":{"message":"x"}},{"sha":"bbb"}]"#;
        assert_eq!(parse_pr_commits(gh), Some(vec!["aaa".to_string(), "bbb".to_string()]));
        let glab = r#"[{"id":"ccc","short_id":"ccc"}]"#;
        assert_eq!(parse_pr_commits(glab), Some(vec!["ccc".to_string()]));
        assert_eq!(parse_pr_commits("[]"), Some(Vec::new()));
        assert_eq!(parse_pr_commits(r#"{"message":"Not Found"}"#), None);
        assert_eq!(parse_pr_commits("<html>"), None);
    }

    #[test]
    fn the_transcript_detector_reads_a_create_and_its_result() {
        let mut pending = Vec::new();
        let call = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"opening"},
            {"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"cd /x && gh pr create --fill"}}]}}"#;
        assert!(pr_create_line(call, &mut pending).is_empty());
        assert_eq!(pending, ["toolu_1"]);
        let view = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_2","name":"Bash",
            "input":{"command":"gh pr view 123 --json url"}}]}}"#;
        pr_create_line(view, &mut pending);
        assert_eq!(pending, ["toolu_1"], "a view is a mention");
        let other = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_2",
            "content":"https://github.com/o/r/pull/123"}]}}"#;
        assert!(pr_create_line(other, &mut pending).is_empty(), "a result of a view binds nothing");
        let result = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_1",
            "content":[{"type":"text","text":"Creating pull request for fix/x into main\n\nhttps://github.com/o/r/pull/42\n"}]}]}}"#;
        assert_eq!(pr_create_line(result, &mut pending), ["https://github.com/o/r/pull/42"]);
        assert!(pending.is_empty());
        // Codex's rollout.
        let fc = r#"{"type":"response_item","payload":{"type":"function_call","name":"shell","call_id":"c1",
            "arguments":"{\"command\":[\"bash\",\"-lc\",\"glab mr create --fill --yes\"]}"}}"#;
        pr_create_line(fc, &mut pending);
        assert_eq!(pending, ["c1"]);
        let fo = r#"{"type":"response_item","payload":{"type":"function_call_output","call_id":"c1",
            "output":"https://gitlab.example.com/g/p/-/merge_requests/9\n"}}"#;
        assert_eq!(
            pr_create_line(fo, &mut pending),
            ["https://gitlab.example.com/g/p/-/merge_requests/9"]
        );
        assert!(pr_create_line("not json", &mut pending).is_empty());
        assert!(is_pr_create("/usr/local/bin/gh pr create -t x"));
        assert!(!is_pr_create("gh pr list"));
        for i in 0..20 {
            remember(&mut pending, &format!("id{i}"));
        }
        assert_eq!(
            pending.len(),
            PENDING_CAP,
            "a result that never comes does not grow the cursor"
        );
    }

    #[test]
    fn pr_urls_are_found_in_text_and_numbered() {
        let urls = pr_urls("done: <https://github.com/o/r/pull/42>, and https://gitlab.com/g/s/p/-/merge_requests/7. https://github.com/o/r/issues/3");
        assert_eq!(
            urls,
            ["https://github.com/o/r/pull/42", "https://gitlab.com/g/s/p/-/merge_requests/7"]
        );
        assert_eq!(pr_number_of("https://github.com/o/r/pull/42"), Some((Forge::GitHub, 42)));
        assert_eq!(
            pr_number_of("https://gitlab.com/g/s/p/-/merge_requests/7"),
            Some((Forge::GitLab, 7))
        );
        assert_eq!(pr_number_of("https://github.com/o/r/pull/x"), None);
        // A Codex result kept as a JSON string spells its newline (T-795).
        assert_eq!(
            pr_urls(r#"{"output":"https://github.com/o/r/pull/5\n","metadata":{}}"#),
            ["https://github.com/o/r/pull/5"]
        );
        assert!(same_url("https://a/b/pull/1/", "https://a/b/pull/1"));
        let r = repo_of_url("https://github.com/o/r/pull/42").unwrap();
        assert_eq!((r.host.as_str(), r.path.as_str()), ("github.com", "o/r"));
        let r = repo_of_url("https://gitlab.example.com/g/s/p/-/merge_requests/7").unwrap();
        assert_eq!((r.host.as_str(), r.path.as_str()), ("gitlab.example.com", "g/s/p"));
        assert_eq!(repo_of_url("https://github.com/o/r/issues/3"), None);
        assert_eq!(Forge::GitLab.name(), "GitLab");
    }

    #[test]
    fn deployments_read_by_environment_and_names_are_bounded() {
        let ds = parse_gh_deployments(
            r#"[{"id":7,"environment":"staging","sha":"m"},{"id":8,"environment":"production"}]"#,
        );
        assert_eq!(ds, [(7, "staging".to_string()), (8, "production".to_string())]);
        assert_eq!(
            parse_gh_deployment_success(r#"[{"state":"success"},{"state":"pending"}]"#),
            Some(0)
        );
        assert_eq!(
            parse_gh_deployment_success(
                r#"[{"state":"success","created_at":"2026-10-15T00:00:05Z"}]"#
            ),
            Some(iso_ms(Some(&Value::from("2026-10-15T00:00:05Z"))))
        );
        assert_eq!(parse_gh_deployment_success(r#"[{"state":"in_progress"}]"#), None);
        assert_eq!(
            parse_gh_deployment_success(
                r#"[{"state":"inactive","created_at":"2026-10-15T00:09:00Z"},
                    {"state":"success","created_at":"2026-10-15T00:00:05Z"},{"state":"in_progress"}]"#
            ),
            Some(iso_ms(Some(&Value::from("2026-10-15T00:00:05Z")))),
            "a deployment a newer one superseded was deployed, when it succeeded (T-800)"
        );
        assert_eq!(
            parse_gh_deployment_success(r#"[{"state":"inactive"},{"state":"failure"}]"#),
            None,
            "inactive alone is no deployment"
        );
        assert_eq!(parse_gh_deployment_success("[]"), None);
        assert_eq!(parse_gitlab_deployed(r#"[{"sha":"abc","status":"success"}]"#, "abc"), Some(0));
        assert!(parse_gitlab_deployed(
            r#"[{"sha":"abc","finished_at":"2026-10-15T00:00:05Z"}]"#,
            "abc"
        )
        .is_some_and(|ms| ms > 0));
        assert!(
            parse_gitlab_deployed(r#"[{"deployable":{"commit":{"id":"abc"}}}]"#, "abc").is_some()
        );
        assert_eq!(parse_gitlab_deployed(r#"[{"sha":"zzz"}]"#, "abc"), None);
        assert_eq!(parse_gitlab_deployed(r#"[{"sha":""}]"#, ""), None);
        let long = "x".repeat(200);
        assert_eq!(job_word(&format!("  a\nb\t{long}")).len(), NAME_MAX_BYTES);
        assert_eq!(job_word("test\nlinux"), "test linux");
    }

    #[test]
    fn iso_stamps_read_as_unix_ms() {
        let v = |s: &str| Value::String(s.into());
        assert_eq!(iso_ms(Some(&v("1970-01-01T00:00:00Z"))), 0);
        assert_eq!(iso_ms(Some(&v("2026-10-13T05:56:17Z"))), 1_791_870_977_000);
        assert_eq!(iso_ms(Some(&v("2026-10-13T07:56:17+02:00"))), 1_791_870_977_000);
        assert_eq!(iso_ms(Some(&v("2026-10-13T05:56:17.123Z"))), 1_791_870_977_000);
        assert_eq!(iso_ms(Some(&v("nope"))), 0);
        assert_eq!(iso_ms(None), 0);
        assert_eq!(Pipeline::from_words("failed", "t"), Pipeline::Failed("t".into()));
        assert_eq!(Pipeline::from_words("later", ""), Pipeline::Unknown);
        assert_eq!(PrState::from_word("open"), Some(PrState::Open));
        assert_eq!(Review::from_word("approved"), Review::Approved);
        assert_eq!(Forge::from_word("gitlab"), Some(Forge::GitLab));
    }

    /// The entry rule (T-750): a bound PR is judged by its pipeline and
    /// unknown is never green; with none bound, a PR opened here waits for
    /// the forge and a ticket that opened none has nothing to pass.
    #[test]
    fn entry_needs_a_passed_pipeline_only_where_a_pr_is_or_was_opened() {
        let with = |pipeline: Pipeline| {
            let mut p = pr("o/r", 1, PrState::Open, "aa", 1);
            p.pipeline = pipeline;
            vec![p]
        };
        // Bound: today's judgement, whatever the transcript says.
        let failed = with(Pipeline::Failed("test".into()));
        assert_eq!(
            entry_refusal(Some(&failed), Some(false)).as_deref(),
            Some("pipeline failed: test")
        );
        assert_eq!(entry_refusal(Some(&with(Pipeline::Passed)), Some(true)), None);
        assert_eq!(
            entry_refusal(Some(&with(Pipeline::Running("build".into()))), None).as_deref(),
            Some("pipeline running")
        );
        // Bound and the forge unreachable: its reading is unknown, and a
        // ticket that opened nothing here is refused all the same.
        let mut merged = pr("o/r", 1, PrState::Merged, "aa", 1);
        merged.pipeline = Pipeline::Unknown;
        assert_eq!(
            entry_refusal(Some(&[merged]), Some(false)).as_deref(),
            Some("pipeline not read")
        );
        let open = with(Pipeline::Unknown);
        assert_eq!(
            entry_refusal(Some(&open), Some(false)).as_deref(),
            Some("pipeline not read"),
            "an open PR unread is never green"
        );
        // Nor beside one that passed (T-795): the fold does not drop it.
        let mut two = with(Pipeline::Passed);
        let mut web = pr("o/web", 1, PrState::Open, "bb", 1);
        web.pipeline = Pipeline::Unknown;
        two.push(web);
        assert_eq!(pipeline_of(&two), Pipeline::Unknown);
        assert_eq!(entry_refusal(Some(&two), Some(false)).as_deref(), Some("pipeline not read"));
        two[0].pipeline = Pipeline::Failed("test".into());
        assert_eq!(
            pipeline_of(&two),
            Pipeline::Failed("test".into()),
            "a failure still says itself"
        );
        // Unbound, a URL opened here: the forge has not read it yet.
        assert_eq!(entry_refusal(None, Some(true)).as_deref(), Some("pipeline not read"));
        assert_eq!(entry_refusal(Some(&[]), Some(true)).as_deref(), Some("pipeline not read"));
        // Unbound and the transcript unreadable: unknown, never green.
        assert_eq!(entry_refusal(None, None).as_deref(), Some("pipeline not read"));
        // Unbound and nothing opened here: nothing to pass.
        assert_eq!(entry_refusal(None, Some(false)), None);
        assert_eq!(entry_refusal(Some(&[]), Some(false)), None);
        // T-797: a closed PR alone is none bound. Nothing unread, it enters;
        // a URL a read still owes holds it, as with no PR at all.
        let closed = ticket_set(vec![pr("o/r", 1, PrState::Closed, "aa", 1)]);
        assert_eq!(entry_refusal(Some(&closed), Some(false)), None);
        assert_eq!(entry_refusal(Some(&closed), Some(true)).as_deref(), Some("pipeline not read"));
        // Beside an open one, the open one is judged.
        let mut both = with(Pipeline::Running(String::new()));
        both.extend(closed);
        assert_eq!(entry_refusal(Some(&both), Some(false)).as_deref(), Some("pipeline running"));
    }

    /// T-797: a URL names its PR by the same URL, or by its number and the
    /// repository's path in any case; another number, another repository or
    /// a URL that names no PR does not.
    #[test]
    fn a_printed_url_names_its_pr_by_number_and_path() {
        let gh = "https://github.com/Example/Forge/pull/42";
        assert!(url_names(gh, "example/forge", 42));
        assert!(url_names(gh, "Example/Forge", 42));
        assert!(!url_names(gh, "example/forge", 43));
        assert!(!url_names(gh, "example/other", 42));
        let gl = "https://gitlab.com/g/sub/p/-/merge_requests/3";
        assert!(url_names(gl, "g/sub/p", 3));
        assert!(!url_names("https://github.com/example/forge/issues/42", "example/forge", 42));
    }

    /// T-780: MR !5 passed at `a`, the worker pushed `b`, and the forge has
    /// not read it: the pass is the commit before's, so the head is unread.
    /// At the tip it is read; another branch's PR, a repository with no leg
    /// and a branch git no longer has are not judged.
    #[test]
    fn a_pr_whose_head_is_not_the_tip_is_unread() {
        let mut p = pr("g/r", 5, PrState::Open, "a", 1);
        p.head_ref = "msmn/T-9-x".into();
        p.pipeline = Pipeline::Passed;
        let tips = |t: &str| vec![("g/r".to_string(), t.to_string())];
        assert!(head_unread(std::slice::from_ref(&p), "msmn/T-9-x", &tips("b")));
        assert!(!head_unread(std::slice::from_ref(&p), "msmn/T-9-x", &tips("a")));
        assert!(!head_unread(std::slice::from_ref(&p), "msmn/T-9-x", &tips("")));
        assert!(!head_unread(std::slice::from_ref(&p), "msmn/T-9-x", &[]));
        assert!(!head_unread(std::slice::from_ref(&p), "msmn/T-1-y", &tips("b")));
        let mut merged = p.clone();
        merged.state = PrState::Merged;
        assert!(
            !head_unread(&[merged], "msmn/T-9-x", &tips("b")),
            "a merged PR's is not the gate's"
        );
    }

    /// T-774: a branch merged by its first MR, then a second MR opened from
    /// another branch on the same ticket. The open one holds the merge
    /// whichever repository or branch it is on, and says so in the merge
    /// refusal's words.
    #[test]
    fn an_open_pr_on_a_merged_ticket_holds_the_merge() {
        let gl = |mut p: Pr, head_ref: &str| {
            p.forge = Forge::GitLab;
            p.head_ref = head_ref.into();
            p
        };
        let first = gl(pr("g/app", 1, PrState::Merged, "tip1", 10), "msmn/A1-x");
        let second = gl(pr("g/app", 2, PrState::Open, "tip2", 20), "fix/greeting");
        let set = ticket_set(vec![first.clone(), second.clone()]);
        let held = holding(&set).expect("the open MR holds");
        assert_eq!(held.number, 2);
        assert_eq!(open_refusal(held), "MR !2 open — merge it on GitLab");

        // On another repository: the branch's own repo reads merged at its
        // tip, and the ticket is held all the same.
        let other = gl(pr("g/api", 3, PrState::Draft, "tip3", 30), "fix/api");
        let set = ticket_set(vec![first.clone(), other]);
        assert!(merged_at(&set, &["g/app".into()], "tip1").is_some());
        assert_eq!(holding(&set).map(|p| p.number), Some(3), "a draft is open");

        // Merged, the follow-up holds nothing.
        let mut merged = second;
        merged.state = PrState::Merged;
        let landed = ticket_set(vec![first, merged]);
        assert_eq!(holding(&landed), None);

        // T-778: nothing open, the merge is still the forge's.
        assert_eq!(
            merge_refusal(&landed).as_deref(),
            Some("its pull requests carry the merge — merge on GitLab")
        );
        assert_eq!(merge_refusal(&set).as_deref(), Some("MR !3 open — merge it on GitLab"));
        assert_eq!(merge_refusal(&[]), None, "no PR: the board's merge");
        assert_eq!(merge_why(&[]), None);
        assert_eq!(merge_why(&landed), Some("its work goes through pull requests"));

        // T-797: closed unmerged, every one, the newest is named and its
        // reopening is the road back; one merged beside it is the forge's
        // merge as before.
        let closed = |n: u64, at: u64| gl(pr("g/app", n, PrState::Closed, "tip", at), "fix/x");
        let abandoned = ticket_set(vec![closed(4, 40), closed(5, 50)]);
        assert_eq!(
            merge_refusal(&abandoned).as_deref(),
            Some("MR !5 closed unmerged — reopen it on GitLab")
        );
        assert_eq!(
            merge_why(&abandoned),
            Some("its pull requests were closed unmerged on the forge")
        );
        let mut both = landed.clone();
        both.push(closed(6, 60));
        let both = ticket_set(both);
        assert_eq!(
            merge_refusal(&both).as_deref(),
            Some("its pull requests carry the merge — merge on GitLab")
        );
        assert_eq!(merge_why(&both), Some("its work goes through pull requests"));
    }
}
