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
    /// Closed without merging: abandoned, and dropped from a ticket's set.
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
/// run, a run a pass, so the worst of several is the fold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pipeline {
    /// Not asked yet, or the forge did not answer: never read as green.
    Unknown,
    /// The forge ran nothing for this commit.
    None,
    Passed,
    Running,
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
            Pipeline::Running => "running",
            Pipeline::Waiting(_) => "waiting",
            Pipeline::Failed(_) => "failed",
        }
    }

    /// The job the word is about, where there is one.
    pub fn job(&self) -> Option<&str> {
        match self {
            Pipeline::Waiting(j) | Pipeline::Failed(j) if !j.is_empty() => Some(j),
            _ => None,
        }
    }

    pub fn from_words(word: &str, job: Option<&str>) -> Pipeline {
        let job = job.unwrap_or_default().to_string();
        match word {
            "none" => Pipeline::None,
            "passed" => Pipeline::Passed,
            "running" => Pipeline::Running,
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
            Pipeline::Running => 3,
            Pipeline::Waiting(_) => 4,
            Pipeline::Failed(_) => 5,
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
    /// The environments the merge commit reached, as the forge names them;
    /// read only where a column asks for one, and only once merged.
    pub deployed: Vec<String>,
}

impl Pr {
    pub fn label(&self) -> String {
        self.forge.label(self.number)
    }
}

/// Read `gh pr list --json …`'s array. A line that is not that shape reads
/// as no PRs, never as an error: a forge's words are untrusted text and a
/// surprise in them must not take the sample down.
pub fn parse_gh(repo: &str, json: &str) -> Vec<Pr> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(json) else { return Vec::new() };
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
                url: str_of(v, "url"),
                state,
                head_oid: str_of(v, "headRefOid"),
                head_ref: str_of(v, "headRefName"),
                merge_oid,
                conflict: v.get("mergeable").and_then(Value::as_str) == Some("CONFLICTING"),
                review,
                pipeline: gh_rollup(v.get("statusCheckRollup")),
                updated_ms: iso_ms(v.get("updatedAt")),
                deployed: Vec::new(),
            })
        })
        .collect()
}

/// The one `pipeline` word for a PR's `statusCheckRollup`: the worst of its
/// check runs and status contexts, a failing one named.
pub fn gh_rollup(rollup: Option<&Value>) -> Pipeline {
    let Some(Value::Array(entries)) = rollup else { return Pipeline::None };
    let mut fold = Pipeline::None;
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
            (_, "FAILURE" | "TIMED_OUT" | "STARTUP_FAILURE" | "CANCELLED", _)
            | (_, _, "FAILURE" | "ERROR") => Pipeline::Failed(name),
            (_, "ACTION_REQUIRED", _) | ("WAITING", _, _) => Pipeline::Waiting(name),
            ("COMPLETED", _, _) | (_, _, "SUCCESS") => Pipeline::Passed,
            // Queued, in progress, pending, requested, expected.
            _ => Pipeline::Running,
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
    items
        .iter()
        .filter_map(|v| {
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
            let merge_oid = ["merge_commit_sha", "squash_commit_sha"]
                .iter()
                .filter_map(|k| v.get(k).and_then(Value::as_str))
                .find(|s| !s.is_empty())
                .unwrap_or_default()
                .to_string();
            let status = v.get("detailed_merge_status").and_then(Value::as_str).unwrap_or_default();
            let conflict = v.get("has_conflicts").and_then(Value::as_bool) == Some(true)
                || status == "conflict";
            let review = match status {
                "not_approved" => Review::Required,
                _ => Review::None,
            };
            Some(Pr {
                forge: Forge::GitLab,
                repo: repo.to_string(),
                number,
                url: str_of(v, "web_url"),
                state,
                head_oid: str_of(v, "sha"),
                head_ref: str_of(v, "source_branch"),
                merge_oid,
                conflict,
                review,
                pipeline: Pipeline::Unknown,
                updated_ms: iso_ms(v.get("updated_at")),
                deployed: Vec::new(),
            })
        })
        .collect()
}

/// The one `pipeline` word for GitLab's `pipelines?sha=` answer: the newest
/// pipeline's status. GitLab names no failing job on this road, so a
/// failure reads bare.
pub fn parse_gitlab_pipelines(json: &str) -> Pipeline {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(json) else {
        return Pipeline::Unknown;
    };
    let Some(first) = items.first() else { return Pipeline::None };
    gitlab_status(first.get("status").and_then(Value::as_str).unwrap_or_default())
}

/// A GitLab pipeline status word as the board's.
pub fn gitlab_status(status: &str) -> Pipeline {
    match status {
        "success" => Pipeline::Passed,
        "failed" | "canceled" | "cancelled" => Pipeline::Failed(String::new()),
        "manual" => Pipeline::Waiting(String::new()),
        "skipped" => Pipeline::None,
        "created" | "waiting_for_resource" | "preparing" | "pending" | "running" | "scheduled" => {
            Pipeline::Running
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

/// A string field, scrubbed: the forge's words are untrusted text and this
/// is the boundary they cross (`text::scrub_text`).
fn str_of(v: &Value, key: &str) -> String {
    crate::text::scrub_text(v.get(key).and_then(Value::as_str).unwrap_or_default())
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

/// GitHub's `deployments/{id}/statuses?per_page=1` answer: whether the
/// newest status is a success.
pub fn parse_gh_deployment_success(json: &str) -> bool {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(json) else { return false };
    items.first().and_then(|s| s.get("state")).and_then(Value::as_str) == Some("success")
}

/// GitLab's `deployments?environment=&status=success` answer: whether one
/// of them deployed this commit.
pub fn parse_gitlab_deployed(json: &str, oid: &str) -> bool {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(json) else { return false };
    !oid.is_empty()
        && items.iter().any(|d| {
            d.get("sha").and_then(Value::as_str) == Some(oid)
                || d.get("deployable")
                    .and_then(|x| x.get("commit"))
                    .and_then(|c| c.get("id"))
                    .and_then(Value::as_str)
                    == Some(oid)
        })
}

/// An ISO-8601 stamp (`2026-10-13T05:56:17Z`, or with an offset) as unix
/// ms; zero where it is not one. Whole seconds are what both forges send.
pub fn iso_ms(v: Option<&Value>) -> u64 {
    let Some(s) = v.and_then(Value::as_str) else { return 0 };
    let b = s.as_bytes();
    if b.len() < 19 {
        return 0;
    }
    let num = |from: usize, to: usize| s.get(from..to).and_then(|x| x.parse::<i64>().ok());
    let (Some(y), Some(mo), Some(d), Some(h), Some(mi), Some(sec)) =
        (num(0, 4), num(5, 7), num(8, 10), num(11, 13), num(14, 16), num(17, 19))
    else {
        return 0;
    };
    // Days from the civil date (Howard Hinnant's algorithm).
    let (y2, mo2) = if mo <= 2 { (y - 1, mo + 9) } else { (y, mo - 3) };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * mo2 + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let mut secs = days * 86_400 + h * 3600 + mi * 60 + sec;
    // A trailing offset, `+02:00` / `-0500`; `Z` or none is UTC.
    let rest = &s[19..];
    let rest = rest
        .split_once('.')
        .map_or(rest, |(_, after)| after.trim_start_matches(|c: char| c.is_ascii_digit()));
    if let Some(sign) = rest.chars().next().filter(|c| *c == '+' || *c == '-') {
        let digits: String = rest[1..].chars().filter(|c| c.is_ascii_digit()).collect();
        if digits.len() == 4 {
            let oh: i64 = digits[0..2].parse().unwrap_or(0);
            let om: i64 = digits[2..4].parse().unwrap_or(0);
            let off = oh * 3600 + om * 60;
            secs = if sign == '+' { secs - off } else { secs + off };
        }
    }
    u64::try_from(secs).map(|s| s * 1000).unwrap_or(0)
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
        Some(Remote { host: host.to_ascii_lowercase(), path: path.to_string() })
    }

    /// The forge the host is known to be, before any CLI is asked:
    /// github.com is GitHub; any other host is asked of both CLIs.
    pub fn known_forge(&self) -> Option<Forge> {
        (self.host == "github.com").then_some(Forge::GitHub)
    }
}

/// The facts a ticket offers for binding: what its worktree legs hold and
/// what its sessions printed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facts {
    /// The ticket's own `msmn/` branch.
    pub branch: String,
    /// Every commit a worktree leg's HEAD ever pointed at (its reflog), by
    /// the leg's repository path as the remote spells it.
    pub reflog: Vec<(String, String)>,
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
        && facts.reflog.iter().any(|(repo, oid)| *repo == pr.repo && *oid == pr.head_oid)
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

/// A ticket's set out of the bound PRs: open or merged ones, newest first;
/// a closed unmerged PR is abandoned and drops out.
pub fn ticket_set(mut prs: Vec<Pr>) -> Vec<Pr> {
    prs.retain(|p| p.state != PrState::Closed);
    prs.sort_by(|a, b| b.updated_ms.cmp(&a.updated_ms).then(b.number.cmp(&a.number)));
    prs.dedup_by(|a, b| a.repo == b.repo && a.number == b.number);
    prs
}

/// The set's merged PR for one repository at one tip, if the newest PR
/// there is merged and its head is that tip: a commit after the merge is
/// work that has not landed, as the git road reads it.
pub fn merged_at<'a>(set: &'a [Pr], repo: &str, tip: &str) -> Option<&'a Pr> {
    let newest = set.iter().find(|p| p.repo == repo)?;
    (newest.state == PrState::Merged && !tip.is_empty() && newest.head_oid == tip).then_some(newest)
}

/// The worst pipeline over the set's open PRs; the merged commit's once
/// nothing is open and one is merged (the caller has read it onto the PR);
/// `Unknown` with no PR.
pub fn pipeline_of(set: &[Pr]) -> Pipeline {
    let open: Vec<&Pr> = set.iter().filter(|p| p.state.is_open()).collect();
    if !open.is_empty() {
        return open.iter().fold(Pipeline::None, |acc, p| acc.worst(p.pipeline.clone()));
    }
    match set.iter().find(|p| p.state == PrState::Merged) {
        Some(p) => p.pipeline.clone(),
        None => Pipeline::Unknown,
    }
}

/// Why a column that needs a passed pipeline refuses a ticket (T-739), or
/// `None` when it may enter. `set` is the forge's last reading of the
/// ticket's PRs; `opened_here` whether its transcripts opened one, `None`
/// when they could not be read. A ticket with no PR bound and none opened
/// has no pipeline to pass and enters, as a ticket with no branch passes
/// the merge rule (T-750); one opened here and not yet read by the forge
/// waits for it, so a turn's end seconds after a `pr create` is not waved
/// through. Unknown is never green.
pub fn entry_refusal(set: Option<&[Pr]>, opened_here: Option<bool>) -> Option<String> {
    let set = set.filter(|s| !s.is_empty());
    if set.is_none() && opened_here == Some(false) {
        return None;
    }
    match set.map(pipeline_of).unwrap_or(Pipeline::Unknown) {
        Pipeline::Passed => None,
        Pipeline::Unknown => Some("pipeline not read".into()),
        Pipeline::None => Some("no pipeline ran".into()),
        Pipeline::Running => Some("pipeline running".into()),
        Pipeline::Waiting(job) if job.is_empty() => Some("pipeline waiting".into()),
        Pipeline::Waiting(job) => Some(format!("pipeline waiting: {job}")),
        Pipeline::Failed(job) if job.is_empty() => Some("pipeline failed".into()),
        Pipeline::Failed(job) => Some(format!("pipeline failed: {job}")),
    }
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
        let url = word[at..].trim_end_matches(['.', ',', ';', ':', ')', ']']);
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
            deployed: Vec::new(),
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
    }

    #[test]
    fn the_rollup_folds_to_the_loudest_word() {
        let v = |s: &str| serde_json::from_str::<Value>(s).unwrap();
        let running = v(
            r#"[{"name":"a","status":"QUEUED"},{"name":"b","status":"COMPLETED","conclusion":"SUCCESS"}]"#,
        );
        assert_eq!(gh_rollup(Some(&running)), Pipeline::Running);
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
        assert_eq!(
            parse_gitlab_pipelines(r#"[{"status":"manual"}]"#),
            Pipeline::Waiting(String::new())
        );
        assert_eq!(
            parse_gitlab_pipelines(r#"[{"status":"success"},{"status":"failed"}]"#),
            Pipeline::Passed
        );
        assert_eq!(parse_gitlab_pipelines("[]"), Pipeline::None);
        assert_eq!(gitlab_status("running"), Pipeline::Running);
        assert_eq!(gitlab_status("failed"), Pipeline::Failed(String::new()));
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

    #[test]
    fn a_pr_binds_by_a_fact_and_never_by_a_mention() {
        let facts = Facts {
            branch: "msmn/T-12-x".into(),
            reflog: vec![("o/r".into(), "aaa".into()), ("o/r".into(), "bbb".into())],
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
    fn the_set_is_newest_first_without_the_abandoned() {
        let set = ticket_set(vec![
            pr("o/r", 1, PrState::Merged, "a", 10),
            pr("o/r", 2, PrState::Closed, "b", 20),
            pr("o/r", 3, PrState::Open, "c", 30),
            pr("o/r", 3, PrState::Open, "c", 30),
        ]);
        assert_eq!(set.iter().map(|p| p.number).collect::<Vec<_>>(), [3, 1]);
        assert_eq!(merged_at(&set, "o/r", "c"), None, "the newest is open");
        let landed = ticket_set(vec![pr("o/r", 1, PrState::Merged, "a", 10)]);
        assert_eq!(merged_at(&landed, "o/r", "a").map(|p| p.number), Some(1));
        assert_eq!(merged_at(&landed, "o/r", "a2"), None, "a commit after the merge is unlanded");
        assert_eq!(merged_at(&landed, "o/r", ""), None);
        let mut running = pr("o/r", 4, PrState::Open, "d", 40);
        running.pipeline = Pipeline::Running;
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
        assert!(parse_gh_deployment_success(r#"[{"state":"success"},{"state":"pending"}]"#));
        assert!(!parse_gh_deployment_success(r#"[{"state":"in_progress"}]"#));
        assert!(!parse_gh_deployment_success("[]"));
        assert!(parse_gitlab_deployed(r#"[{"sha":"abc","status":"success"}]"#, "abc"));
        assert!(parse_gitlab_deployed(r#"[{"deployable":{"commit":{"id":"abc"}}}]"#, "abc"));
        assert!(!parse_gitlab_deployed(r#"[{"sha":"zzz"}]"#, "abc"));
        assert!(!parse_gitlab_deployed(r#"[{"sha":""}]"#, ""));
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
        assert_eq!(Pipeline::from_words("failed", Some("t")), Pipeline::Failed("t".into()));
        assert_eq!(Pipeline::from_words("later", None), Pipeline::Unknown);
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
            entry_refusal(Some(&with(Pipeline::Running)), None).as_deref(),
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
        assert!(
            entry_refusal(Some(&open), Some(false)).is_some(),
            "an open PR unread is never green"
        );
        // Unbound, a URL opened here: the forge has not read it yet.
        assert_eq!(entry_refusal(None, Some(true)).as_deref(), Some("pipeline not read"));
        assert_eq!(entry_refusal(Some(&[]), Some(true)).as_deref(), Some("pipeline not read"));
        // Unbound and the transcript unreadable: unknown, never green.
        assert_eq!(entry_refusal(None, None).as_deref(), Some("pipeline not read"));
        // Unbound and nothing opened here: nothing to pass.
        assert_eq!(entry_refusal(None, Some(false)), None);
        assert_eq!(entry_refusal(Some(&[]), Some(false)), None);
    }
}
