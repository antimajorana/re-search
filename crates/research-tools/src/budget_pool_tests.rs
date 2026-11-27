use super::*;
use crate::run_budget::IDEA_EXCHANGE_EVENT;
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Barrier;
use std::thread;

fn ev(seq: u64, kind: &str, payload: serde_json::Value) -> Event {
    Event {
        session: "s-pool".to_string(),
        seq,
        ts_ms: (seq as i64) * 1000,
        kind: kind.to_string(),
        schema_version: 1,
        payload,
        prev_hash: ledger::GENESIS_HASH.to_string(),
        event_hash: ledger::GENESIS_HASH.to_string(),
    }
}

#[test]
fn initializes_from_events_and_enforces_frontier_reserve() {
    // 1 main_loop + 1 delegated call (deduped) = 2 committed on Open (cap 10, reserve 2).
    let events = vec![
        ev(1, "investigation.task_requested", json!({ "task": "open", "cap": 10 })),
        ev(2, "model.request", json!({ "turn": 1, "run_id": "1" })),
        ev(3, "delegate.start", json!({ "call_id": "d1", "run_id": "1" })),
        ev(4, "delegate.turn", json!({ "call_id": "d1", "turn": 1, "run_id": "1" })),
        ev(5, "delegate.end", json!({ "call_id": "d1", "turns": 1, "run_id": "1" })),
    ];
    let pool = RunBudgetPool::from_events(
        &events,
        RunScope::AfterSeq(1),
        10,
        DEFAULT_FRONTIER_RESERVE,
        0,
    );
    let snap = pool.snapshot();
    assert_eq!(snap.committed, 2);
    assert_eq!(snap.available_for_workers, 6);
    assert_eq!(snap.available_for_frontier, 8);

    // Workers acquire and commit exactly 6 slots (reaching 8/10), leaving 2 for Frontier.
    for i in 0..6 {
        let lease = pool
            .try_acquire(PoolCaller::worker("paper_reader", Some(format!("r{i}"))))
            .expect("worker slot within headroom");
        lease.mark_sent();
    }

    let err = pool
        .try_acquire(PoolCaller::worker("paper_reader", Some("r6".into())))
        .err()
        .expect("7th worker call must not encroach on frontier_reserve");
    assert_eq!(err.scope, ExhaustedScope::ReservedHeadroom);
    assert_eq!(err.committed, 8);
    assert_eq!(err.reserved, 2);

    // Frontier can still execute its 2 closing calls (record_map_reading + EndTurn).
    pool.try_acquire(PoolCaller::frontier()).unwrap().mark_sent();
    pool.try_acquire(PoolCaller::frontier()).unwrap().mark_sent();

    let end_err = pool.try_acquire(PoolCaller::frontier()).err().unwrap();
    assert_eq!(end_err.scope, ExhaustedScope::TotalCap);
    assert_eq!(pool.snapshot().committed, 10);
}

#[test]
fn drop_before_mark_sent_refunds_slot_while_mark_sent_is_permanent() {
    let pool = RunBudgetPool::from_events(&[], RunScope::All, 4, DEFAULT_FRONTIER_RESERVE, 0);

    // Simulate local prepare_request failure: lease drops without mark_sent.
    {
        let lease = pool.try_acquire(PoolCaller::worker("compute_worker", None)).unwrap();
        assert_eq!(pool.snapshot().in_flight, 1);
        assert_eq!(pool.snapshot().available_for_workers, 1);
        drop(lease);
    }
    let after_drop = pool.snapshot();
    assert_eq!(after_drop.in_flight, 0);
    assert_eq!(after_drop.committed, 0);
    assert_eq!(after_drop.available_for_workers, 2);

    // Simulate HTTP send start followed by 500/cancel: mark_sent keeps slot committed.
    let lease1 = pool.try_acquire(PoolCaller::worker("compute_worker", None)).unwrap();
    let committed1 = lease1.mark_sent();
    assert_eq!(committed1.snapshot.committed, 1);
    assert_eq!(committed1.snapshot.in_flight, 0);

    let lease2 = pool.try_acquire(PoolCaller::worker("compute_worker", None)).unwrap();
    lease2.mark_sent();

    assert!(pool
        .try_acquire(PoolCaller::worker("compute_worker", None))
        .is_err());
    assert_eq!(pool.snapshot().available_for_frontier, 2);
}

#[test]
fn enforces_per_item_sub_caps_for_explore_papers() {
    let events = vec![
        ev(1, IDEA_EXCHANGE_EVENT, json!({ "stage": "sharpen", "attempt": 1 })),
    ];
    let mut sub_caps = BTreeMap::new();
    sub_caps.insert("arxiv:2501.00001".to_string(), SubCap::new(3));
    sub_caps.insert("arxiv:2501.00002".to_string(), SubCap::new(3));

    let pool = RunBudgetPool::from_events(
        &events,
        RunScope::All,
        24,
        EXPLORE_FRONTIER_RESERVE,
        0,
    )
    .with_sub_caps(sub_caps);

    // Paper 1 uses 3 calls and hits its per-paper sub-cap cleanly.
    for _ in 0..3 {
        pool.try_acquire(PoolCaller::worker(
            "idea_reader",
            Some("arxiv:2501.00001".into()),
        ))
        .unwrap()
        .mark_sent();
    }
    let sub_err = pool
        .try_acquire(PoolCaller::worker(
            "idea_reader",
            Some("arxiv:2501.00001".into()),
        ))
        .err()
        .expect("paper 1 sub-cap exhausted");
    assert_eq!(
        sub_err.scope,
        ExhaustedScope::SubCap("arxiv:2501.00001".into())
    );

    // Paper 2 still has all 3 calls available.
    let p2 = pool
        .try_acquire(PoolCaller::worker(
            "idea_reader",
            Some("arxiv:2501.00002".into()),
        ))
        .expect("paper 2 remains unaffected");
    p2.mark_sent();
    assert_eq!(pool.snapshot().sub_caps["arxiv:2501.00002"].committed, 1);
}

#[test]
fn solve_reserves_protect_both_verifier_and_frontier_closing() {
    // Solve budget: cap 60, committed 50, frontier_reserve 2, verifier_reserve 5 -> workers get 3.
    let mut events = Vec::new();
    for seq in 1..=50 {
        events.push(ev(seq, "model.request", json!({ "turn": seq })));
    }
    let pool = RunBudgetPool::from_events(
        &events,
        RunScope::All,
        60,
        DEFAULT_FRONTIER_RESERVE,
        SOLVE_VERIFIER_RESERVE,
    );
    assert_eq!(pool.snapshot().available_for_workers, 3);

    for _ in 0..3 {
        pool.try_acquire(PoolCaller::worker("compute_worker", None))
            .unwrap()
            .mark_sent();
    }
    assert_eq!(
        pool.try_acquire(PoolCaller::worker("compute_worker", None))
            .err()
            .unwrap()
            .scope,
        ExhaustedScope::ReservedHeadroom
    );

    // Verifier runs all 5 closing review calls while preserving frontier_reserve = 2.
    for _ in 0..5 {
        pool.try_acquire(PoolCaller::verifier("verifier"))
            .unwrap()
            .mark_sent();
    }
    assert_eq!(
        pool.try_acquire(PoolCaller::verifier("verifier"))
            .err()
            .unwrap()
            .scope,
        ExhaustedScope::ReservedHeadroom
    );

    // Frontier consumes the last 2 calls for solve_finish + EndTurn.
    assert_eq!(pool.snapshot().unspent_verifier_reserve, 0);
    assert_eq!(pool.snapshot().available_for_frontier, 2);
    pool.try_acquire(PoolCaller::frontier()).unwrap().mark_sent();
    pool.try_acquire(PoolCaller::frontier()).unwrap().mark_sent();
    assert_eq!(pool.snapshot().committed, 60);
}

#[test]
fn concurrent_parallel_workers_never_breach_frontier_reserve_or_total_cap() {
    // 15 racing threads contend across a barrier while 1 worker holds an in-flight lease
    // against total_cap = 15, frontier_reserve = 2 (13 max worker slots).
    let pool = RunBudgetPool::from_events(&[], RunScope::All, 15, DEFAULT_FRONTIER_RESERVE, 0);
    let in_flight_lease = pool
        .try_acquire(PoolCaller::worker("paper_reader", Some("in-flight".into())))
        .expect("initial in-flight worker lease");
    assert_eq!(pool.snapshot().in_flight, 1);
    assert_eq!(pool.snapshot().available_for_workers, 12);

    let threads = 15usize;
    let barrier = Arc::new(Barrier::new(threads));
    let granted = Arc::new(AtomicU64::new(0));
    let rejected = Arc::new(AtomicU64::new(0));

    thread::scope(|s| {
        for tid in 0..threads {
            let p = pool.clone();
            let b = Arc::clone(&barrier);
            let g = Arc::clone(&granted);
            let r = Arc::clone(&rejected);
            s.spawn(move || {
                b.wait();
                for _attempt in 0..4 {
                    match p.try_acquire(PoolCaller::worker("paper_reader", Some(format!("t{tid}"))))
                    {
                        Ok(lease) => {
                            lease.mark_sent();
                            g.fetch_add(1, Ordering::SeqCst);
                        }
                        Err(err) => {
                            assert_eq!(err.scope, ExhaustedScope::ReservedHeadroom);
                            r.fetch_add(1, Ordering::SeqCst);
                        }
                    }
                }
            });
        }
    });

    // While `in_flight_lease` stayed open, racing threads could claim at most 12 slots.
    let mid = pool.snapshot();
    assert_eq!(granted.load(Ordering::SeqCst), 12);
    assert_eq!(rejected.load(Ordering::SeqCst), (threads * 4) as u64 - 12);
    assert_eq!(mid.committed, 12);
    assert_eq!(mid.in_flight, 1);
    assert_eq!(mid.available_for_workers, 0);

    // Dropping the in-flight lease (local prepare failure) refunds exactly 1 worker slot.
    drop(in_flight_lease);
    let after_drop = pool.snapshot();
    assert_eq!(after_drop.in_flight, 0);
    assert_eq!(after_drop.available_for_workers, 1);

    pool.try_acquire(PoolCaller::worker("paper_reader", Some("recovered".into())))
        .unwrap()
        .mark_sent();

    let snap = pool.snapshot();
    assert_eq!(snap.committed, 13);
    assert_eq!(snap.available_for_workers, 0);
    assert_eq!(snap.available_for_frontier, 2);

    // Frontier closing pass still gets both reserved calls.
    pool.try_acquire(PoolCaller::frontier()).unwrap().mark_sent();
    pool.try_acquire(PoolCaller::frontier()).unwrap().mark_sent();
    assert_eq!(pool.snapshot().committed, 15);
    assert!(pool.try_acquire(PoolCaller::frontier()).is_err());
}

#[test]
fn concurrent_explore_workers_respect_both_per_paper_sub_caps_and_global_pool() {
    // 6 papers * 3 calls_per_paper = 18 possible, but total_cap = 16 with explore reserve = 6 -> 10 max!
    let pool = RunBudgetPool::from_events(
        &[],
        RunScope::All,
        16,
        EXPLORE_FRONTIER_RESERVE,
        0,
    )
    .with_default_sub_cap(3);

    let barrier = Arc::new(Barrier::new(6));
    let total_sent = Arc::new(AtomicU64::new(0));

    thread::scope(|s| {
        for paper_idx in 0..6 {
            let p = pool.clone();
            let b = Arc::clone(&barrier);
            let ts = Arc::clone(&total_sent);
            s.spawn(move || {
                let key = format!("paper-{paper_idx}");
                b.wait();
                for _ in 0..5 {
                    match p.try_acquire(PoolCaller::worker("idea_reader", Some(key.clone()))) {
                        Ok(lease) => {
                            lease.mark_sent();
                            ts.fetch_add(1, Ordering::SeqCst);
                        }
                        Err(_) => break,
                    }
                }
            });
        }
    });

    let snap = pool.snapshot();
    assert_eq!(total_sent.load(Ordering::SeqCst), 10);
    assert_eq!(snap.committed, 10);
    assert_eq!(snap.available_for_workers, 0);
    assert_eq!(snap.available_for_frontier, EXPLORE_FRONTIER_RESERVE);
    for sub in snap.sub_caps.values() {
        assert!(sub.committed <= 3, "paper exceeded calls_per_paper=3: {sub:?}");
    }

    // Raising the cap unlocks additional worker headroom cleanly without losing history.
    pool.raise_cap(4);
    assert_eq!(pool.snapshot().available_for_workers, 4);
    assert_eq!(pool.to_model_call_budget().cap, 20);
}
