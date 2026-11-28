use super::*;
use crate::paper_key::{PaperIds, PaperKey, PaperRef};
use std::collections::BTreeSet;

fn make_hit(paper: PaperRef, title: &str) -> RawHit {
    RawHit {
        paper,
        title: title.to_string(),
        authors: vec![],
        year: None,
        abstract_text: None,
        abstract_verbatim: false,
        abstract_raw_blob: None,
        citation_count: None,
        withdrawn_or_retracted: false,
        in_library: false,
        owned: false,
        user_seed: false,
        cited_by_origin: false,
    }
}

#[test]
fn merges_arxiv_and_doi_aliases_with_f2_canonical_key() {
    let h1 = RawHit {
        year: Some(2021),
        citation_count: Some(12),
        abstract_text: Some("Verbatim arXiv abstract.".into()),
        abstract_verbatim: true,
        abstract_raw_blob: Some("blob-arxiv".into()),
        ..make_hit(
            PaperRef::from_ids(
                PaperIds {
                    arxiv: Some("2103.01234v2".into()),
                    doi: Some("10.1000/alpha".into()),
                    ..Default::default()
                },
                None,
            )
            .unwrap(),
            "Krylov Growth",
        )
    };
    let h2 = RawHit {
        year: Some(2022),
        citation_count: Some(45),
        owned: true,
        abstract_text: Some("Much longer rebuilt abstract that is not verbatim from index.".into()),
        abstract_verbatim: false,
        abstract_raw_blob: Some("blob-rebuilt".into()),
        ..make_hit(
            PaperRef::from_ids(PaperIds { doi: Some("10.1000/ALPHA".into()), ..Default::default() }, None)
                .unwrap(),
            "Krylov Growth in Monitored Chains",
        )
    };

    let lists = vec![
        RankedServiceList {
            qid: "q-1".into(),
            provider: "arxiv".into(),
            perspective_id: Some("p-1".into()),
            hits: vec![h1],
        },
        RankedServiceList {
            qid: "q-2".into(),
            provider: "crossref".into(),
            perspective_id: Some("p-2".into()),
            hits: vec![h2],
        },
    ];

    let ranked = merge_and_rank(&lists);
    assert_eq!(ranked.len(), 1);
    let c = &ranked[0];
    assert_eq!(c.key.as_str(), "arxiv:2103.01234");
    assert_eq!(c.version.as_deref(), Some("v2"));
    assert_eq!(c.ids.arxiv.as_deref(), Some("2103.01234"));
    assert_eq!(c.ids.doi.as_deref(), Some("10.1000/alpha"));
    assert_eq!(c.title, "Krylov Growth in Monitored Chains");
    assert_eq!(c.year, Some(2021));
    assert_eq!(c.citation_count, Some(45));
    assert!(c.owned);
    assert_eq!(c.found_by, vec!["q-1", "q-2"]);
    assert_eq!(c.perspectives, vec!["p-1", "p-2"]);
    // Verbatim abstract beats longer rebuilt abstract.
    assert_eq!(c.abstract_text.as_deref(), Some("Verbatim arXiv abstract."));
    assert_eq!(
        c.abstract_ref,
        Some(AbstractRef {
            raw_blob: Some("blob-arxiv".into()),
            provider: "arxiv".into(),
            verbatim: true,
        })
    );

    let expected_score = (1.0 / (RRF_K + 1.0))
        + (1.0 / (RRF_K + 1.0))
        + 2.0 * PERSPECTIVE_BONUS
        + (46.0_f64).ln() * CITATION_LOG_WEIGHT;
    assert!((c.score - expected_score).abs() < 1e-12);
}

#[test]
fn merges_four_way_transitive_alias_chain() {
    let s2_id = "0123456789abcdef0123456789abcdef01234567";
    let h_ax_doi = make_hit(
        PaperRef::from_ids(
            PaperIds { arxiv: Some("2305.00001".into()), doi: Some("10.1000/chain".into()), ..Default::default() },
            Some("v1".into()),
        )
        .unwrap(),
        "Short",
    );
    let h_doi_ins = make_hit(
        PaperRef::from_ids(
            PaperIds { doi: Some("10.1000/chain".into()), inspire: Some("1234567".into()), ..Default::default() },
            None,
        )
        .unwrap(),
        "Medium Title",
    );
    let h_ins_s2 = make_hit(
        PaperRef::from_ids(
            PaperIds { inspire: Some("1234567".into()), s2: Some(s2_id.into()), ..Default::default() },
            None,
        )
        .unwrap(),
        "Full Transitive Chain Title",
    );

    let lists = vec![
        RankedServiceList { qid: "q-3".into(), provider: "semantic_scholar".into(), perspective_id: None, hits: vec![h_ins_s2] },
        RankedServiceList { qid: "q-1".into(), provider: "arxiv".into(), perspective_id: None, hits: vec![h_ax_doi] },
        RankedServiceList { qid: "q-2".into(), provider: "inspire".into(), perspective_id: None, hits: vec![h_doi_ins] },
    ];
    let ranked = merge_and_rank(&lists);
    assert_eq!(ranked.len(), 1);
    let c = &ranked[0];
    assert_eq!(c.key.as_str(), "arxiv:2305.00001");
    assert_eq!(c.ids.arxiv.as_deref(), Some("2305.00001"));
    assert_eq!(c.ids.doi.as_deref(), Some("10.1000/chain"));
    assert_eq!(c.ids.inspire.as_deref(), Some("1234567"));
    assert_eq!(c.ids.s2.as_deref(), Some(s2_id));
    assert_eq!(c.found_by, vec!["q-1", "q-2", "q-3"]);
    assert_eq!(c.title, "Full Transitive Chain Title");
}

#[test]
fn canonical_key_follows_f2_kind_order_not_lexicographic_prefix() {
    // Lexicographically "file:" < "inspire:" < "s2:", but F2 puts Inspire and S2 before File.
    let file_key = PaperKey::from_file_bytes(b"synthetic-pdf-bytes");
    let h_file = make_hit(
        PaperRef {
            key: file_key,
            version: None,
            ids: PaperIds { inspire: Some("999111".into()), ..Default::default() },
        },
        "Local File Copy",
    );
    let h_ins = make_hit(
        PaperRef::from_ids(PaperIds { inspire: Some("999111".into()), ..Default::default() }, None).unwrap(),
        "INSPIRE Record",
    );
    let lists = vec![RankedServiceList {
        qid: "q-1".into(),
        provider: "inspire".into(),
        perspective_id: None,
        hits: vec![h_file, h_ins],
    }];
    let ranked = merge_and_rank(&lists);
    assert_eq!(ranked.len(), 1);
    assert_eq!(ranked[0].key.as_str(), "inspire:999111");
    // Same list duplicate takes best rank (1), not 1 + 2.
    let expected = 1.0 / (RRF_K + 1.0);
    assert!((ranked[0].score - expected).abs() < 1e-12);
}

#[test]
fn caps_candidates_at_200_authors_at_8_and_selects_highest_numeric_version() {
    let mut hits = Vec::new();
    for i in 1..=230usize {
        let mut h = make_hit(
            PaperRef::from_ids(
                PaperIds { arxiv: Some(format!("2201.{i:05}")), ..Default::default() },
                Some(if i == 1 { "v2".into() } else { "v1".into() }),
            )
            .unwrap(),
            &format!("Paper {i}"),
        );
        h.authors = (1..=12).map(|a| format!("Author {a}")).collect();
        hits.push(h);
    }
    let v10 = make_hit(
        PaperRef::from_ids(PaperIds { arxiv: Some("2201.00001".into()), ..Default::default() }, Some("v10".into()))
            .unwrap(),
        "Paper 1",
    );
    let lists = vec![
        RankedServiceList { qid: "q-1".into(), provider: "arxiv".into(), perspective_id: None, hits },
        RankedServiceList { qid: "q-2".into(), provider: "inspire".into(), perspective_id: None, hits: vec![v10] },
    ];

    let capped = merge_and_rank(&lists);
    assert_eq!(capped.len(), MAX_CANDIDATES_PER_ROUND);
    assert_eq!(capped[0].key.as_str(), "arxiv:2201.00001");
    assert_eq!(capped[0].version.as_deref(), Some("v10"));
    assert_eq!(capped[0].authors.len(), MAX_AUTHORS);

    let uncapped = merge_and_rank_with_cap(&lists, None);
    assert_eq!(uncapped.len(), 230);
}

// --- Deterministic property-style tests (SplitMix64, zero external dependencies) ---

struct SplitMix64(u64);

impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn gen_range(&mut self, upper: usize) -> usize {
        (self.next_u64() as usize) % upper
    }

    fn shuffle<T>(&mut self, slice: &mut [T]) {
        for i in (1..slice.len()).rev() {
            let j = self.gen_range(i + 1);
            slice.swap(i, j);
        }
    }
}

fn generate_case(rng: &mut SplitMix64) -> Vec<RankedServiceList> {
    let n_papers = 4 + rng.gen_range(12);
    let providers = ["arxiv", "inspire", "semantic_scholar", "crossref"];
    let perspectives = [None, Some("p-1"), Some("p-2"), Some("p-3")];

    // Pre-build a full alias identity table per underlying paper.
    let mut full_ids: Vec<PaperIds> = Vec::new();
    for p in 0..n_papers {
        let arxiv = (rng.gen_range(10) < 7).then(|| format!("2301.{:05}", p + 1));
        let doi = (rng.gen_range(10) < 8 || arxiv.is_none()).then(|| format!("10.5555/p{}", p + 1));
        let inspire = (rng.gen_range(10) < 6).then(|| format!("{}", 100_000 + p));
        let s2 = (rng.gen_range(10) < 5).then(|| format!("{:040x}", p + 1));
        full_ids.push(PaperIds { arxiv, doi, inspire, s2 });
    }

    let n_lists = 2 + rng.gen_range(5);
    let mut lists = Vec::with_capacity(n_lists);
    for l in 0..n_lists {
        let n_hits = 1 + rng.gen_range(n_papers + 2);
        let mut hits = Vec::with_capacity(n_hits);
        for _ in 0..n_hits {
            let p = rng.gen_range(n_papers);
            let full = &full_ids[p];
            // Randomly expose a non-empty subset of the paper's identifiers (creating alias merges).
            let mut ids = PaperIds::default();
            if rng.gen_range(2) == 0 { ids.arxiv = full.arxiv.clone(); }
            if rng.gen_range(2) == 0 { ids.doi = full.doi.clone(); }
            if rng.gen_range(2) == 0 { ids.inspire = full.inspire.clone(); }
            if rng.gen_range(2) == 0 { ids.s2 = full.s2.clone(); }
            if ids.arxiv.is_none() && ids.doi.is_none() && ids.inspire.is_none() && ids.s2.is_none() {
                if let Some(v) = &full.doi { ids.doi = Some(v.clone()); }
                else if let Some(v) = &full.arxiv { ids.arxiv = Some(v.clone()); }
            }
            let version = ids.arxiv.as_ref().map(|_| format!("v{}", 1 + rng.gen_range(4)));
            let paper = PaperRef::from_ids(ids, version).unwrap();
            let mut hit = make_hit(paper, &format!("Title {p} var {}", rng.gen_range(4)));
            hit.year = Some(2015 + rng.gen_range(11) as i32);
            hit.citation_count = (rng.gen_range(3) > 0).then(|| rng.next_u64() % 500);
            hit.abstract_text = (rng.gen_range(3) > 0).then(|| format!("Abs {p} len {}", rng.gen_range(3)));
            hit.abstract_verbatim = rng.gen_range(2) == 0;
            hit.abstract_raw_blob = hit.abstract_text.as_ref().map(|_| format!("blob-{}", rng.gen_range(5)));
            hit.owned = rng.gen_range(5) == 0;
            hit.user_seed = rng.gen_range(7) == 0;
            hit.withdrawn_or_retracted = rng.gen_range(10) == 0;
            hits.push(hit);
        }
        lists.push(RankedServiceList {
            qid: format!("q-{}", l + 1),
            provider: providers[rng.gen_range(providers.len())].to_string(),
            perspective_id: perspectives[rng.gen_range(perspectives.len())].map(str::to_string),
            hits,
        });
    }
    lists
}

#[test]
fn property_service_list_permutation_invariance_is_bit_exact() {
    let mut rng = SplitMix64(0x5eed_cafe_1234_5678);
    for case in 0..120 {
        let lists = generate_case(&mut rng);
        let baseline = merge_and_rank_with_cap(&lists, None);

        let mut shuffled = lists.clone();
        rng.shuffle(&mut shuffled);
        let permuted = merge_and_rank_with_cap(&shuffled, None);

        assert_eq!(baseline.len(), permuted.len(), "case {case} length mismatch");
        for (a, b) in baseline.iter().zip(permuted.iter()) {
            assert_eq!(a.key, b.key, "case {case} key mismatch");
            assert_eq!(a.score.to_bits(), b.score.to_bits(), "case {case} float ULP mismatch");
            assert_eq!(a, b, "case {case} merged candidate mismatch");
        }
    }
}

#[test]
fn property_merge_is_order_independent_idempotent_and_loses_no_hit() {
    let mut rng = SplitMix64(0xdead_beef_9876_5432);
    for case in 0..120 {
        let lists = generate_case(&mut rng);
        let merged = merge_and_rank_with_cap(&lists, None);

        // 1. No hit is lost: every input key and alias appears in some output candidate.
        let mut output_keys: BTreeSet<PaperKey> = BTreeSet::new();
        for c in &merged {
            for k in collect_canonical_keys(&c.paper_ref()) {
                assert!(
                    output_keys.insert(k.clone()),
                    "case {case}: key `{k}` appeared in two distinct output candidates"
                );
            }
        }
        for list in &lists {
            for hit in &list.hits {
                for k in collect_canonical_keys(&hit.paper) {
                    assert!(output_keys.contains(&k), "case {case}: lost input key `{k}`");
                }
            }
        }

        // 2. Order independence of pure hit merging (all hits flattened into one list, shuffled).
        let flat: Vec<RawHit> = lists.iter().flat_map(|l| l.hits.iter().cloned()).collect();
        let mut flat_rev = flat.clone();
        rng.shuffle(&mut flat_rev);
        let m1 = merge_and_rank_with_cap(
            &[RankedServiceList { qid: "q-flat".into(), provider: "arxiv".into(), perspective_id: None, hits: flat }],
            None,
        );
        let m2 = merge_and_rank_with_cap(
            &[RankedServiceList { qid: "q-flat".into(), provider: "arxiv".into(), perspective_id: None, hits: flat_rev }],
            None,
        );
        assert_eq!(m1.len(), m2.len(), "case {case} component count differs");
        let strip_score = |v: &[RankedCandidate]| -> Vec<RankedCandidate> {
            let mut rows: Vec<RankedCandidate> = v
                .iter()
                .cloned()
                .map(|mut c| {
                    c.score = 0.0;
                    c
                })
                .collect();
            rows.sort_by(|a, b| a.key.cmp(&b.key));
            rows
        };
        assert_eq!(strip_score(&m1), strip_score(&m2), "case {case} metadata merge not order-independent");

        // 3. Idempotence: merging an already-merged candidate list with itself keeps exact keys & metadata.
        let as_hits: Vec<RawHit> = m1.iter().map(RankedCandidate::to_raw_hit).collect();
        let mut doubled = as_hits.clone();
        doubled.extend(as_hits);
        let once = merge_and_rank_with_cap(
            &[RankedServiceList { qid: "q-id".into(), provider: "arxiv".into(), perspective_id: None, hits: m1.iter().map(RankedCandidate::to_raw_hit).collect() }],
            None,
        );
        let twice = merge_and_rank_with_cap(
            &[RankedServiceList { qid: "q-id".into(), provider: "arxiv".into(), perspective_id: None, hits: doubled }],
            None,
        );
        assert_eq!(once, twice, "case {case} merge not idempotent");
    }
}
