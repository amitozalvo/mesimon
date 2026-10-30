use super::*;

#[test]
fn a_merge_refusal_reaches_the_feed_and_ticket_only_while_current() {
    let dir = std::env::temp_dir().join(format!("msmn-t1049-{}", std::process::id()));
    let (mut d, _, _rx) = queued_person_push_tests::daemon(&dir);
    let id = d.board.sessions[0].ticket;
    d.board.sessions[0].confidence = Confidence::High;
    d.board.merge_into = MergeInto::Main;
    d.board.tickets.push(
        serde_json::from_value(serde_json::json!({
            "id": id, "short_key": "T-1", "title": "worker", "column": "REVIEW",
            "order": "0", "created_at": "@0"
        }))
        .unwrap(),
    );
    d.worktrees.insert(
        id,
        Binding {
            path: dir.join("worker"),
            branch: "msmn/worker".into(),
            base_oid: "base".into(),
            branch_oid: "tip".into(),
            status: BindingStatus::Attached,
            locked: false,
            repos: Vec::new(),
            init: None,
        },
    );
    d.wt_agg.insert(
        id,
        worktree::Aggregate {
            ahead: 1,
            tip: "tip".into(),
            base_tip: "base".into(),
            ..Default::default()
        },
    );
    let (socket, _peer) = UnixStream::pair().unwrap();
    let client = Arc::new(Mutex::new(socket));
    d.train.arm(&client, false);
    let raw = format!(
        "git merge failed:\n\terror: untracked café.txt\u{1b}\u{202e}\n{}",
        "é".repeat(3000)
    );
    d.record_train_refusal(id, "tip".into(), &raw);
    d.feed.flush().unwrap();
    let line: serde_json::Value = std::fs::read_to_string(d.paths.activity_log())
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .find(|line| line["cmd"] == "merge_train_refused:merge")
        .unwrap();
    assert_eq!(line["actor"], "automation");
    assert_eq!(line["ticket"], id.to_string());
    let detail = line["outcome"].as_str().unwrap();
    assert!(detail.starts_with("git merge failed: error: untracked café.txt "));
    assert!(detail.len() <= 2048 && detail.ends_with('…'));
    assert_eq!(mesimon_core::text::scrub_text(detail), detail);
    assert_eq!(d.merge_by(id).by, "train");
    assert_eq!(d.merge_by(id).why, detail);

    // Either tip moving invalidates the reason without needing another attempt.
    for base in [false, true] {
        let a = d.wt_agg.get_mut(&id).unwrap();
        if base {
            a.base_tip = "new base".into();
        } else {
            a.tip = "new tip".into();
        }
        assert_ne!(d.merge_by(id).why, detail);
        let a = d.wt_agg.get_mut(&id).unwrap();
        a.tip = "tip".into();
        a.base_tip = "base".into();
    }
    d.board.crown_mode = CrownMode::Supervised;
    assert_eq!(d.merge_by(id).by, "person");
    assert_ne!(d.merge_by(id).why, detail);
    d.board.crown_mode = CrownMode::Autonomous;
    d.board.ticket_mut(id).unwrap().manual_merge = true;
    assert_eq!(d.merge_by(id).by, "crown");
    assert_ne!(d.merge_by(id).why, detail);
    d.board.ticket_mut(id).unwrap().manual_merge = false;
    d.board.sessions[0].state = SessionState::Running;
    assert_eq!(d.merge_by(id).by, "train");
    assert_ne!(d.merge_by(id).why, detail);
    d.board.sessions[0].state = SessionState::Idle { stop_reason: StopReason::EndTurn };
    d.train.disarm();
    assert_eq!(d.merge_by(id).by, "crown");
    assert_ne!(d.merge_by(id).why, detail);
    d.train.arm(&client, false);
    d.wt_agg.get_mut(&id).unwrap().merged = true;
    assert_eq!(d.merge_by(id).by, "nobody");
    assert_ne!(d.merge_by(id).why, detail);
    d.wt_agg.get_mut(&id).unwrap().merged = false;
    assert_eq!(d.merge_by(id).why, detail);
    d.train.forget_refusals();
    assert_ne!(d.merge_by(id).why, detail, "a changed checkout clears its reason");
    drop(d);
    std::fs::remove_dir_all(dir).unwrap();
}
