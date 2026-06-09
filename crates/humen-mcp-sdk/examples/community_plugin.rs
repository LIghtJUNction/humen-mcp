use humen_mcp_sdk::{
    HumenPluginManifest, HumenTaskKind, RequestTemplate, RouteStrategy, ScoringRule,
    ThirdPartyChannel,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest = HumenPluginManifest {
        id: "community-review".to_string(),
        name: "Community Review".to_string(),
        version: "0.1.0".to_string(),
        description: "Adds review-oriented templates and routing hints.".to_string(),
        author: "community".to_string(),
        request_templates: vec![RequestTemplate {
            id: "release-review".to_string(),
            title: "Release review".to_string(),
            description: "Ask a human to review a release before publishing.".to_string(),
            kind: HumenTaskKind::Judgment,
            prompt_template: "Review this release plan and decide whether it can ship.".to_string(),
            choices: vec!["ship".to_string(), "hold".to_string()],
            steps: vec!["Check risk".to_string(), "Check rollback plan".to_string()],
            tags: vec!["#release".to_string()],
            timeout_seconds: Some(300),
        }],
        route_strategies: vec![RouteStrategy {
            id: "trusted-online".to_string(),
            title: "Trusted online humans".to_string(),
            description: "Prefer online humans with a strong reputation.".to_string(),
            tags: vec!["#release".to_string()],
            prefer_online: true,
            minimum_reputation: Some(7),
        }],
        scoring_rules: vec![ScoringRule {
            id: "risk-rubric".to_string(),
            title: "Risk rubric".to_string(),
            description: "Score release risk with explicit checks.".to_string(),
            weight: 1.0,
            rubric: vec!["Rollback exists".to_string(), "Tests passed".to_string()],
        }],
        channels: vec![ThirdPartyChannel {
            id: "webhook-review".to_string(),
            title: "Webhook review channel".to_string(),
            description: "Forward review tasks to an external webhook.".to_string(),
            kind: "webhook".to_string(),
            endpoint: Some("https://example.invalid/humen".to_string()),
            config_schema: Default::default(),
        }],
        ..Default::default()
    };

    manifest.validate()?;
    println!("{}", serde_json::to_string_pretty(&manifest)?);
    Ok(())
}
