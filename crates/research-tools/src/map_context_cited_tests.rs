use super::*;
use crate::skeleton::CITED_STATUS_FOUND;

#[test]
fn rule_6_9_cited_result_checks_lookup_and_external_node_containment() {
    let f = Fixture::new();
    let thm_tag = Fixture::tag_by_label(&f.skeleton, "thm:decay");
    let ext_thm_tag = Fixture::tag_by_label(&f.ext_skeleton, "thm:ext");
    let req_seq = f.start_task("context", &[&thm_tag]);
    f.supply_both_sources(req_seq, CALLER);

    // Record a candidates-only lookup for Uncited99 (not located) and found lookup for Smith24.
    f.record_cited_lookup("n-fake", "Uncited99", None, "candidates");
    f.record_cited_lookup(&thm_tag, "Smith24", Some(&ext_thm_tag), CITED_STATUS_FOUND);

    let ext_lem_line = Fixture::line_in(EXTERNAL_TEX, "local coercivity");
    let ext_thm_h_line = Fixture::line_in(EXTERNAL_TEX, "boundary weight is continuous");
    let ext_thm_c_line = Fixture::line_in(EXTERNAL_TEX, "semigroup decays exponentially");
    let pri_thm_line = Fixture::line_in(PRIMARY_TEX, "external contraction estimate");

    let res = record_context_cards(
        &f.env,
        &f.ledger,
        SID,
        CALLER,
        &json!({
            "request_seq": req_seq,
            "source_id": f.source_id,
            "tag": thm_tag,
            "cards": [
                {
                    "type": "cited_result",
                    "title": "Valid Smith24 Theorem",
                    "body": "States exponential semigroup decay under continuous boundary weight.",
                    "bib_key": "Smith24",
                    "anchors": [f.ext_anchor(ext_thm_c_line, "semigroup decays exponentially in $L^2$.")],
                    "hypotheses": [
                        {
                            "text": "Spectral gap lower bound & continuous weight",
                            "anchor": f.ext_anchor(ext_thm_h_line, "boundary weight is continuous")
                        }
                    ]
                },
                {
                    "type": "cited_result",
                    "title": "Missing lookup",
                    "body": "Uncited99 is candidates-only.",
                    "bib_key": "Uncited99",
                    "anchors": [f.ext_anchor(ext_thm_c_line, "semigroup decays exponentially")]
                },
                {
                    "type": "cited_result",
                    "title": "Wrong external node",
                    "body": "Anchored in ext lemma instead of ext theorem.",
                    "bib_key": "Smith24",
                    "anchors": [f.ext_anchor(ext_lem_line, "local coercivity inequality")]
                },
                {
                    "type": "cited_result",
                    "title": "Hypothesis outside located node",
                    "body": "Main anchor ok, hypothesis in wrong node.",
                    "bib_key": "Smith24",
                    "anchors": [f.ext_anchor(ext_thm_c_line, "semigroup decays exponentially")],
                    "hypotheses": [
                        {
                            "text": "Wrong place",
                            "anchor": f.ext_anchor(ext_lem_line, "local coercivity inequality")
                        }
                    ]
                },
                {
                    "type": "cited_result",
                    "title": "Primary-only anchor",
                    "body": "Primary anchor without external node anchor.",
                    "bib_key": "Smith24",
                    "anchors": [f.primary_anchor(pri_thm_line, "external contraction estimate")]
                }
            ]
        }),
    )
    .unwrap();

    assert_eq!(res["status"], "recorded");
    let accepted = res["accepted"].as_array().unwrap();
    assert_eq!(accepted.len(), 1);
    assert_eq!(accepted[0]["origin"], "external");
    assert_eq!(accepted[0]["bib_key"], "Smith24");
    assert_eq!(accepted[0]["external_source_id"], f.ext_source_id);
    assert_eq!(accepted[0]["external_tag"], ext_thm_tag);

    let rejected = res["rejected"].as_array().unwrap();
    assert_eq!(rejected.len(), 4);
    assert_eq!(rejected[0]["code"], "cited_result_not_located");
    assert_eq!(rejected[1]["code"], "anchor_outside_cited_node");
    assert_eq!(rejected[2]["code"], "hypothesis_outside_cited_node");
    assert_eq!(rejected[3]["code"], "anchor_outside_cited_node");
}
