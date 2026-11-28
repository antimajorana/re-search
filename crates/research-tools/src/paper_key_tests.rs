use super::*;
use serde_json::json;

fn key(text: &str) -> PaperKey { PaperKey::parse(text).unwrap() }

#[test]
fn arxiv_new_style_spellings_give_one_key() {
    for raw in [
        "2103.01234",
        "2103.01234v2",
        " arXiv:2103.01234v12 ",
        "arxiv: 2103.01234",
        "https://arxiv.org/abs/2103.01234v2",
        "http://arxiv.org/pdf/2103.01234v2.pdf",
        "https://export.arxiv.org/e-print/2103.01234",
    ] {
        let k = PaperKey::from_arxiv(raw).unwrap();
        assert_eq!(k.as_str(), "arxiv:2103.01234", "{raw}");
        assert_eq!(k.kind(), PaperKeyKind::Arxiv);
        assert_eq!(k.value(), "2103.01234");
    }
    assert_eq!(PaperKey::from_arxiv("0704.0001").unwrap().as_str(), "arxiv:0704.0001");
    assert_eq!(PaperKey::from_arxiv("1412.8765").unwrap().as_str(), "arxiv:1412.8765");
    assert_eq!(PaperKey::from_arxiv("1501.00001").unwrap().as_str(), "arxiv:1501.00001");
}

#[test]
fn arxiv_version_is_split_off() {
    assert_eq!(split_arxiv_id("2103.01234").unwrap(), ("2103.01234".to_string(), None));
    assert_eq!(split_arxiv_id("arXiv:2103.01234V3").unwrap(), ("2103.01234".to_string(), Some("v3".to_string())));
    assert_eq!(split_arxiv_id("hep-th/9901001v2").unwrap(), ("hep-th/9901001".to_string(), Some("v2".to_string())));
    // An archive name containing the letter v must not be mistaken for a version.
    assert_eq!(split_arxiv_id("solv-int/9901001").unwrap(), ("solv-int/9901001".to_string(), None));
}

#[test]
fn arxiv_old_style_drops_subject_class_and_case() {
    for raw in ["math.AG/0601001", "math/0601001", "MATH.ag/0601001v1", "https://arxiv.org/abs/math.AG/0601001"] {
        assert_eq!(PaperKey::from_arxiv(raw).unwrap().as_str(), "arxiv:math/0601001", "{raw}");
    }
    assert_eq!(PaperKey::from_arxiv("HEP-TH/9901001").unwrap().as_str(), "arxiv:hep-th/9901001");
    assert_eq!(PaperKey::from_arxiv("cond-mat.str-el/0703999").unwrap().as_str(), "arxiv:cond-mat/0703999");
    assert_eq!(PaperKey::from_arxiv("hep-th/9108001").unwrap().value(), "hep-th/9108001");
}

#[test]
fn arxiv_rejections() {
    for raw in [
        "", "2103", "2103.123", "2103.012345", "21030.1234", "abc", "2103.01234v0", "2103.01234v",
        "2113.01234",      // month 13
        "2100.01234",      // month 00
        "2103.0123",       // four digits after 1501
        "1412.01234",      // five digits before 1501
        "0703.0001",       // new-style before it existed
        "0612.1234",
        "hep-th/0704001",  // old-style after it ended
        "hep-th/1001001",
        "hep-th/9913001",  // month 13
        "hep-th/990100",   // six digits
        "-th/9901001", "hep-/9901001",
        "math.AG.GT/0601001",
        "10.1000/abc",
    ] {
        assert!(PaperKey::from_arxiv(raw).is_err(), "{raw:?} should be rejected");
    }
}

#[test]
fn doi_is_normalised_like_literature_search() {
    for raw in [
        "10.1103/PhysRevLett.116.061102",
        "  doi:10.1103/PhysRevLett.116.061102 ",
        "DOI: 10.1103/physrevlett.116.061102",
        "https://doi.org/10.1103/PhysRevLett.116.061102",
        "http://dx.doi.org/10.1103/PhysRevLett.116.061102",
        "https://doi.org/10.1103%2FPhysRevLett.116.061102",
    ] {
        let k = PaperKey::from_doi(raw).unwrap();
        assert_eq!(k.as_str(), "doi:10.1103/physrevlett.116.061102", "{raw}");
        assert_eq!(k.kind(), PaperKeyKind::Doi);
        // Same value as the function behind `literature_search` hits.
        assert_eq!(Some(k.value().to_string()), crate::tex_scan::normalize_doi(raw));
    }
    for raw in ["", "11.1000/abc", "10.1000", "10.1000/", "10./abc", "10.1000/a b", "physrevlett.116", "doi:"] {
        assert!(PaperKey::from_doi(raw).is_err(), "{raw:?} should be rejected");
    }
}

#[test]
fn inspire_s2_openalex_forms() {
    assert_eq!(PaperKey::from_inspire(" 1234567 ").unwrap().as_str(), "inspire:1234567");
    for raw in ["", "0", "012", "12a", "-3", "1234567890123"] {
        assert!(PaperKey::from_inspire(raw).is_err(), "{raw:?}");
    }
    let s2 = "649def34f8be52c8b66281af98ae884c09aef38b";
    assert_eq!(PaperKey::from_s2(&s2.to_uppercase()).unwrap().as_str(), format!("s2:{s2}"));
    for raw in ["", "649def34", &s2[1..], &format!("{s2}0"), &s2.replace('6', "g")] {
        assert!(PaperKey::from_s2(raw).is_err(), "{raw:?}");
    }
    for raw in ["W2741809807", "w2741809807", "https://openalex.org/W2741809807", "https://openalex.org/works/W2741809807"] {
        let k = PaperKey::from_openalex(raw).unwrap();
        assert_eq!(k.as_str(), "openalex:W2741809807", "{raw}");
        assert_eq!(k.value(), "W2741809807");
    }
    for raw in ["", "W", "2741809807", "A2741809807", "W012", "W12x", "W 12"] {
        assert!(PaperKey::from_openalex(raw).is_err(), "{raw:?}");
    }
}

#[test]
fn file_key_is_first_16_hex_of_sha256() {
    // sha256("hello\n") = 5891b5b522d5df08..., sha256("") = e3b0c44298fc1c14...
    assert_eq!(PaperKey::from_file_bytes(b"hello\n").as_str(), "file:5891b5b522d5df08");
    assert_eq!(PaperKey::from_file_bytes(b"").as_str(), "file:e3b0c44298fc1c14");
    let k = PaperKey::from_file_bytes(b"%PDF-1.5 some bytes");
    assert_eq!(k.kind(), PaperKeyKind::File);
    assert_eq!(k.value(), &ledger::sha256_hex(b"%PDF-1.5 some bytes")[..16]);
}

#[test]
fn parse_round_trips_every_constructor() {
    let keys = [
        PaperKey::from_arxiv("arXiv:2103.01234v2").unwrap(),
        PaperKey::from_arxiv("math.AG/0601001").unwrap(),
        PaperKey::from_doi("https://doi.org/10.1103/PhysRevLett.116.061102").unwrap(),
        PaperKey::from_inspire("451647").unwrap(),
        PaperKey::from_s2("649def34f8be52c8b66281af98ae884c09aef38b").unwrap(),
        PaperKey::from_openalex("w2741809807").unwrap(),
        PaperKey::from_file_bytes(b"x"),
    ];
    let kinds: Vec<PaperKeyKind> = keys.iter().map(PaperKey::kind).collect();
    assert_eq!(kinds, [
        PaperKeyKind::Arxiv, PaperKeyKind::Arxiv, PaperKeyKind::Doi, PaperKeyKind::Inspire,
        PaperKeyKind::S2, PaperKeyKind::Openalex, PaperKeyKind::File,
    ]);
    for k in keys {
        let again = PaperKey::parse(k.as_str()).unwrap();
        assert_eq!(again, k);
        assert_eq!(again.to_string(), k.as_str());
        assert_eq!(k.as_str().parse::<PaperKey>().unwrap(), k);
        assert_eq!(format!("{}:{}", k.kind().prefix(), k.value()), k.as_str());
        let wire = serde_json::to_value(&k).unwrap();
        assert_eq!(wire, json!(k.as_str()));
        assert_eq!(serde_json::from_value::<PaperKey>(wire).unwrap(), k);
    }
}

#[test]
fn parse_rejects_everything_not_canonical() {
    for text in [
        "", "arxiv", "2103.01234", ":2103.01234", "arxiv:", "sha:5891b5b522d5df08", "isbn:123",
        "arXiv:2103.01234", "ARXIV:2103.01234", " arxiv:2103.01234", "arxiv:2103.01234 ", "arxiv: 2103.01234",
        "arxiv:2103.01234v2", "arxiv:arXiv:2103.01234", "arxiv:https://arxiv.org/abs/2103.01234",
        "arxiv:math.AG/0601001", "arxiv:HEP-TH/9901001", "arxiv:2103.01234.", "arxiv:2103.01234\n",
        "doi:10.1103/PhysRevLett.116.061102", "doi:https://doi.org/10.1000/abc", "doi:doi:10.1000/abc",
        "doi:10.1000/abc.", "doi:10.1000/a b", "doi:10.1000/a%2Fb", "doi:11.1000/abc",
        "inspire:0123", "inspire: 123", "inspire:12a",
        "s2:649DEF34F8BE52C8B66281AF98AE884C09AEF38B", "s2:649def34",
        "openalex:w2741809807", "openalex:2741809807", "openalex:https://openalex.org/W2741809807",
        "file:5891b5b522d5df0", "file:5891b5b522d5df080", "file:5891B5B522D5DF08", "file:5891b5b522d5df0g",
    ] {
        assert!(PaperKey::parse(text).is_err(), "{text:?} should be rejected");
        assert!(serde_json::from_value::<PaperKey>(json!(text)).is_err(), "{text:?} should not deserialise");
    }
    assert!(serde_json::from_value::<PaperKey>(json!(7)).is_err());
}

fn all_ids() -> PaperIds {
    PaperIds {
        arxiv: Some("arXiv:2103.01234v2".into()),
        doi: Some("https://doi.org/10.1103/PhysRevLett.116.061102".into()),
        inspire: Some("451647".into()),
        s2: Some("649DEF34F8BE52C8B66281AF98AE884C09AEF38B".into()),
    }
}

#[test]
fn paper_ref_key_is_first_available_identifier() {
    let openalex = PaperKey::from_openalex("W2741809807").unwrap();
    let expect = ["arxiv:2103.01234", "doi:10.1103/physrevlett.116.061102", "inspire:451647",
        "s2:649def34f8be52c8b66281af98ae884c09aef38b"];
    let mut ids = all_ids();
    for (step, want) in expect.iter().enumerate() {
        // The fallback never overrides one of the four identifiers.
        let r = PaperRef::from_ids_or(ids.clone(), None, Some(openalex.clone())).unwrap();
        assert_eq!(r.key.as_str(), *want);
        assert_eq!(PaperRef::from_ids(ids.clone(), None).unwrap().key, r.key);
        match step {
            0 => ids.arxiv = None,
            1 => ids.doi = None,
            2 => ids.inspire = None,
            _ => ids.s2 = None,
        }
    }
    assert_eq!(ids, PaperIds::default());
    assert!(PaperRef::from_ids(ids.clone(), None).is_err());
    assert!(PaperRef::from_ids(PaperIds { arxiv: Some("  ".into()), ..Default::default() }, None).is_err());
    let r = PaperRef::from_ids_or(ids.clone(), None, Some(openalex.clone())).unwrap();
    assert_eq!(r.key, openalex);
    let file = PaperKey::from_file_bytes(b"dropped");
    assert_eq!(PaperRef::from_ids_or(ids, None, Some(file.clone())).unwrap().key, file);
}

#[test]
fn paper_ref_normalises_ids_and_version() {
    let r = PaperRef::from_ids(all_ids(), None).unwrap();
    assert_eq!(r.version.as_deref(), Some("v2"), "version taken from the arXiv id");
    assert_eq!(r.ids.arxiv.as_deref(), Some("2103.01234"));
    assert_eq!(r.ids.doi.as_deref(), Some("10.1103/physrevlett.116.061102"));
    assert_eq!(r.ids.s2.as_deref(), Some("649def34f8be52c8b66281af98ae884c09aef38b"));
    let r = PaperRef::from_ids(all_ids(), Some(" v3 ".into())).unwrap();
    assert_eq!(r.version.as_deref(), Some("v3"), "an explicit version wins");
    let plain = PaperIds { arxiv: Some("2103.01234".into()), ..Default::default() };
    assert_eq!(PaperRef::from_ids(plain.clone(), None).unwrap().version, None);
    for bad in ["3", "v0", "v03", "version 3", "v3a"] {
        assert!(PaperRef::from_ids(plain.clone(), Some(bad.into())).is_err(), "{bad:?}");
    }
    // A malformed identifier is an error even when an earlier one would give the key.
    let mut ids = all_ids();
    ids.doi = Some("not a doi".into());
    assert!(PaperRef::from_ids(ids, None).is_err());
    // Papers without an arXiv key may carry a non-arXiv version label, but no whitespace.
    let doi_only = PaperIds { doi: Some("10.1000/abc".into()), ..Default::default() };
    assert_eq!(PaperRef::from_ids(doi_only.clone(), Some("2".into())).unwrap().version.as_deref(), Some("2"));
    assert!(PaperRef::from_ids(doi_only, Some("second edition".into())).is_err());
}

#[test]
fn paper_ref_json_shape() {
    let full = PaperRef::from_ids(all_ids(), None).unwrap();
    let wire = serde_json::to_value(&full).unwrap();
    assert_eq!(wire, json!({
        "key": "arxiv:2103.01234",
        "version": "v2",
        "ids": {
            "arxiv": "2103.01234",
            "doi": "10.1103/physrevlett.116.061102",
            "inspire": "451647",
            "s2": "649def34f8be52c8b66281af98ae884c09aef38b"
        }
    }));
    assert_eq!(serde_json::from_value::<PaperRef>(wire).unwrap(), full);

    let small = PaperRef::from_ids(PaperIds { doi: Some("10.1000/ABC".into()), ..Default::default() }, None).unwrap();
    let wire = serde_json::to_value(&small).unwrap();
    assert_eq!(wire, json!({"key": "doi:10.1000/abc", "ids": {"doi": "10.1000/abc"}}));
    assert_eq!(serde_json::to_string(&small).unwrap(), r#"{"key":"doi:10.1000/abc","ids":{"doi":"10.1000/abc"}}"#);
    assert_eq!(serde_json::from_value::<PaperRef>(wire).unwrap(), small);

    let file = PaperRef::from_ids_or(PaperIds::default(), None, Some(PaperKey::from_file_bytes(b"hello\n"))).unwrap();
    assert_eq!(serde_json::to_value(&file).unwrap(), json!({"key": "file:5891b5b522d5df08", "ids": {}}));
    // `ids` may be absent on read; a bad key is refused.
    assert_eq!(serde_json::from_value::<PaperRef>(json!({"key": "file:5891b5b522d5df08"})).unwrap(), file);
    assert!(serde_json::from_value::<PaperRef>(json!({"key": "arxiv:2103.01234v2", "ids": {}})).is_err());
    assert!(serde_json::from_value::<PaperRef>(json!({"ids": {}})).is_err());
}

#[test]
fn node_ref_json_shape_and_mode_names() {
    let bare = NodeRef { session: "s-1".into(), mode: Mode::Solve, id: "e-4".into(), paper: None, node_fp: None };
    let wire = serde_json::to_value(&bare).unwrap();
    assert_eq!(wire, json!({"session": "s-1", "mode": "solve", "id": "e-4"}));
    assert_eq!(serde_json::from_value::<NodeRef>(wire).unwrap(), bare);

    let full = NodeRef {
        session: "0190a1b2".into(),
        mode: Mode::Investigate,
        id: "n-55wblivbkd".into(),
        paper: Some(PaperRef::from_ids(PaperIds { arxiv: Some("1102.0842v3".into()), ..Default::default() }, None).unwrap()),
        node_fp: Some("0123456789ab".into()),
    };
    let wire = serde_json::to_value(&full).unwrap();
    assert_eq!(wire, json!({
        "session": "0190a1b2",
        "mode": "investigate",
        "id": "n-55wblivbkd",
        "paper": {"key": "arxiv:1102.0842", "version": "v3", "ids": {"arxiv": "1102.0842"}},
        "node_fp": "0123456789ab"
    }));
    assert_eq!(serde_json::from_value::<NodeRef>(wire).unwrap(), full);

    for (mode, name) in [(Mode::Investigate, "investigate"), (Mode::Explore, "explore"), (Mode::Solve, "solve"), (Mode::Research, "research")] {
        assert_eq!(serde_json::to_value(mode).unwrap(), json!(name));
        assert_eq!(serde_json::from_value::<Mode>(json!(name)).unwrap(), mode);
        assert_eq!(mode.as_str(), name);
    }
    for bad in ["Investigate", "investigation", "idea", ""] {
        assert!(serde_json::from_value::<Mode>(json!(bad)).is_err(), "{bad:?}");
    }
    assert!(serde_json::from_value::<NodeRef>(json!({"session": "s", "mode": "solve"})).is_err());
}

#[test]
fn id_prefix_table_matches_the_spec() {
    let of = |mode: Mode| -> Vec<&str> { ID_PREFIXES.iter().filter(|e| e.mode == mode).map(|e| e.prefix).collect() };
    assert_eq!(of(Mode::Investigate), ["n-", "m-", "legacy:"]);
    assert_eq!(of(Mode::Explore), ["sc-", "p-", "c-", "d-", "k-", "q-", "w-", "r-", "xe-", "xa-"]);
    assert_eq!(of(Mode::Solve), ["e-"]);
    assert!(of(Mode::Research).is_empty());
    assert_eq!(ID_PREFIXES.len(), 14);
    for entry in ID_PREFIXES {
        assert!(!entry.meaning.is_empty() && !entry.scope.is_empty());
        assert!(entry.prefix.ends_with('-') || entry.prefix.ends_with(':'), "{}", entry.prefix);
    }
    let stable: Vec<&str> = ID_PREFIXES.iter().filter(|e| !e.scope.starts_with("per session")).map(|e| e.prefix).collect();
    assert_eq!(stable, ["n-"], "only the skeleton tag outlives a session");
}

#[test]
fn no_prefix_is_shared_or_starts_another() {
    for (i, a) in ID_PREFIXES.iter().enumerate() {
        for (j, b) in ID_PREFIXES.iter().enumerate() {
            if i == j {
                continue;
            }
            assert_ne!(a.prefix, b.prefix, "prefix reserved twice");
            // Stronger than "no two modes share one": holds inside a mode as well, so a
            // match on the start of an id is never ambiguous.
            assert!(!b.prefix.starts_with(a.prefix), "`{}` is the start of `{}`", a.prefix, b.prefix);
        }
    }
}

#[test]
fn mode_of_id_matches_on_the_start_only() {
    for (id, mode) in [
        ("n-55wblivbkd", Mode::Investigate), ("m-abcdefghij", Mode::Investigate), ("legacy:12:claim_3", Mode::Investigate),
        ("sc-1", Mode::Explore), ("p-2", Mode::Explore), ("c-1", Mode::Explore), ("d-7", Mode::Explore),
        ("k-3", Mode::Explore), ("q-4", Mode::Explore), ("w-5", Mode::Explore), ("r-6", Mode::Explore),
        ("xe-120", Mode::Explore), ("xa-121", Mode::Explore), ("e-9", Mode::Solve),
    ] {
        assert_eq!(mode_of_id(id), Some(mode), "{id}");
    }
    assert_eq!(id_prefix_of("xe-120").unwrap().prefix, "xe-");
    assert_eq!(id_prefix_of("e-120").unwrap().prefix, "e-");
    assert_eq!(id_prefix_of("sc-1").unwrap().prefix, "sc-");
    // No reserved prefix: Solve handles and check ids, bare prefixes, other casing.
    for id in ["", "energy@2", "x@1", "check_dim", "e-", "n-", "legacy:", "N-abc", "legacy", "ye-1", " e-1"] {
        assert_eq!(mode_of_id(id), None, "{id:?}");
    }
}

#[test]
fn base32_matches_rfc_4648_vectors_in_lower_case() {
    for (input, want) in [
        ("", ""), ("f", "my"), ("fo", "mzxq"), ("foo", "mzxw6"), ("foob", "mzxw6yq"),
        ("fooba", "mzxw6ytb"), ("foobar", "mzxw6ytboi"),
    ] {
        assert_eq!(base32_lower_nopad(input.as_bytes()), want, "{input:?}");
    }
    assert_eq!(base32_lower_nopad(&[0xff; 32]).len(), 52);
}

/// Golden values, computed independently (Python `hashlib` + `base64.b32encode`).
/// These must never change: tags are permanent. If this test fails, the code is wrong.
#[test]
fn node_tag_golden_values() {
    assert_eq!(TAG_FIELD_SEPARATOR as u32, 0x1f);
    assert_eq!(node_tag(&key("arxiv:1102.0842"), "theorem", "label:thm:main").unwrap(), "n-55wblivbkd");
    assert_eq!(
        node_tag(&key("doi:10.1103/physrevlett.116.061102"), "section", "fp:0123456789ab#2").unwrap(),
        "n-yyoe2ydq3r"
    );
    assert_eq!(
        node_tag(&PaperKey::from_file_bytes(b"hello\n"), "definition", "label:d\u{e9}f:\u{3b1}").unwrap(),
        "n-oj4khki4t2"
    );
}

#[test]
fn node_tag_shape_and_field_separation() {
    let paper = key("arxiv:1102.0842");
    let tag = node_tag(&paper, "theorem", "label:thm:main").unwrap();
    assert_eq!(tag.len(), 12);
    assert!(tag[2..].bytes().all(|b| matches!(b, b'a'..=b'z' | b'2'..=b'7')));
    assert_eq!(mode_of_id(&tag), Some(Mode::Investigate));
    assert_eq!(node_tag(&paper, "theorem", "label:thm:main").unwrap(), tag, "deterministic");
    // Each field matters, and text cannot move between fields.
    assert_ne!(node_tag(&key("arxiv:1102.0843"), "theorem", "label:thm:main").unwrap(), tag);
    assert_ne!(node_tag(&paper, "lemma", "label:thm:main").unwrap(), tag);
    assert_ne!(node_tag(&paper, "theorem", "label:thm:main2").unwrap(), tag);
    assert_ne!(node_tag(&paper, "theorem", "fp:0123456789ab").unwrap(), node_tag(&paper, "theorem", "fp:0123456789ab#1").unwrap());
    assert_ne!(node_tag(&paper, "a", "b\u{1f}c").unwrap(), node_tag(&paper, "a", "b").unwrap());
    for bad_class in ["", "Theorem", "theorem like", "a\u{1f}b", "a|b"] {
        assert!(node_tag(&paper, bad_class, "label:x").is_err(), "{bad_class:?}");
    }
    assert!(node_tag(&paper, "theorem", "").is_err());
}
