use super::*;

fn role(provider: &str, model: &str) -> RoleModel {
    RoleModel {
        provider: provider.to_string(),
        model: model.to_string(),
    }
}

#[test]
fn compute_review_independence_distinguishes_provider_model_and_same() {
    assert_eq!(
        compute_review_independence(
            &role("anthropic", "claude-sonnet-4-5"),
            &role("openai", "gpt-4.1"),
        ),
        ReviewIndependence::DifferentProvider
    );
    assert_eq!(
        compute_review_independence(
            &role("anthropic", "claude-opus-4-1"),
            &role("anthropic", "claude-sonnet-4-5"),
        ),
        ReviewIndependence::DifferentModel
    );
    assert_eq!(
        compute_review_independence(
            &role("openai", "gpt-4.1"),
            &role("OpenAI", "  GPT-4.1  "),
        ),
        ReviewIndependence::SameModel
    );
    assert_eq!(
        compute_review_independence(
            &role("ollama", "qwen2.5:7b"),
            &role("ollama", "llama3.1:8b"),
        ),
        ReviewIndependence::DifferentModel
    );
    assert_eq!(
        compute_review_independence(
            &role("ollama", "qwen2.5:7b"),
            &role("openai", "gpt-4.1"),
        ),
        ReviewIndependence::DifferentProvider
    );
}

#[test]
fn compute_review_independence_canonicalizes_provider_families_and_prefixes() {
    // Subscription CLI vs direct API for the same underlying provider+model must not claim DifferentProvider.
    assert_eq!(
        compute_review_independence(
            &role("claude-cli", "claude-sonnet-4-5"),
            &role("anthropic", "claude-sonnet-4-5"),
        ),
        ReviewIndependence::SameModel
    );
    assert_eq!(
        compute_review_independence(
            &role("chatgpt", "gpt-5"),
            &role("openai", "gpt-5"),
        ),
        ReviewIndependence::SameModel
    );
    assert_eq!(
        compute_review_independence(
            &role("chatgpt", "gpt-5"),
            &role("openai", "gpt-4.1"),
        ),
        ReviewIndependence::DifferentModel
    );
    assert_eq!(
        compute_review_independence(
            &role("gemini", "models/gemini-2.5-pro"),
            &role("antigravity", "gemini-2.5-pro"),
        ),
        ReviewIndependence::SameModel
    );
    assert_eq!(
        compute_review_independence(
            &role("openai", "openai/gpt-4.1"),
            &role("openai", "gpt-4.1"),
        ),
        ReviewIndependence::SameModel
    );
}

#[test]
fn compute_review_independence_fails_closed_when_ambiguous() {
    assert_eq!(
        compute_review_independence(&role("", "gpt-4.1"), &role("anthropic", "claude-sonnet-4-5")),
        ReviewIndependence::SameModel
    );
    assert_eq!(
        compute_review_independence(&role("openai", "   "), &role("anthropic", "claude-sonnet-4-5")),
        ReviewIndependence::SameModel
    );
    assert_eq!(
        compute_review_independence(&role("openai", "gpt-4.1"), &role("   ", "claude-sonnet-4-5")),
        ReviewIndependence::SameModel
    );
    assert_eq!(
        compute_review_independence(&role("openai", "gpt-4.1"), &role("anthropic", "")),
        ReviewIndependence::SameModel
    );
    assert_eq!(
        compute_review_independence(&role("openai", "openai/"), &role("openai", "gpt-4.1")),
        ReviewIndependence::SameModel
    );
}
