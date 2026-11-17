use pulsar_server::modules::{
    plugins::{
        hooks::{HookDispatcher, HookFailurePolicy, HookStage, RegisteredHook},
        instincts::{AtomicInstinct, FactMemory, MemoryInstinctStore, MemoryProvenance},
    },
    secrets::profiles::{EnvProfile, ProfileManager},
};
use serde_json::json;
use std::sync::Arc;

#[tokio::test]
async fn environment_profiles_bind_and_resolve() {
    let manager = ProfileManager::new();

    let mut vars = std::collections::HashMap::new();
    vars.insert("BASE_URL".into(), "https://staging.api.com".into());
    let mut secrets = std::collections::HashMap::new();
    secrets.insert("STRIPE_KEY".into(), "sk_test_12345".into());

    let profile = EnvProfile {
        id: "staging-profile".into(),
        name: "Staging Environment".into(),
        description: "Staging credentials".into(),
        variables: vars,
        secrets,
        created_at_ms: 1000,
        updated_at_ms: 1000,
    };

    manager.save_profile(profile).await.unwrap();

    // Bind to session
    manager
        .bind_to_session("session-abc", "staging-profile")
        .await
        .unwrap();

    let resolved = manager.resolve_for_session("session-abc").await.unwrap();
    assert_eq!(
        resolved.variables.get("BASE_URL").unwrap(),
        "https://staging.api.com"
    );
    assert_eq!(resolved.secrets.get("STRIPE_KEY").unwrap(), "sk_test_12345");
}

#[tokio::test]
async fn lifecycle_hooks_intercept_and_mutate() {
    let dispatcher = HookDispatcher::new();

    // Register a pre_turn hook that appends a suffix
    dispatcher
        .register_hook(RegisteredHook {
            plugin_id: "guardrail-plugin".into(),
            stage: HookStage::PreTurn,
            policy: HookFailurePolicy::FailClosed,
            handler: Arc::new(|_stage, mut payload| {
                Box::pin(async move {
                    if let Some(text) = payload.get_mut("prompt").and_then(|p| p.as_str()) {
                        let mutated = format!("{text} [verified]");
                        return Ok(Some(json!({ "prompt": mutated })));
                    }
                    Ok(None)
                })
            }),
        })
        .await;

    let payload = json!({ "prompt": "Hello Sovereign Agent" });
    let result = dispatcher
        .dispatch(HookStage::PreTurn, payload)
        .await
        .unwrap();
    assert_eq!(
        result.get("prompt").unwrap(),
        "Hello Sovereign Agent [verified]"
    );
}

#[tokio::test]
async fn memory_provenance_and_instincts_reinforcement() {
    let store = MemoryInstinctStore::new();

    let memory = FactMemory {
        id: "fact-1".into(),
        fact: "User prefers PostgreSQL over MongoDB".into(),
        provenance: MemoryProvenance {
            source_session_id: "sess-99".into(),
            turn_index: 3,
            timestamp_ms: 1700000000,
            author_role: "user".into(),
            source_excerpt: "Always use PostgreSQL for persistent data".into(),
        },
        tags: vec!["preferences".into(), "database".into()],
    };

    store.record_fact(memory).await;
    let fetched = store.get_fact("fact-1").await.unwrap();
    assert_eq!(fetched.provenance.turn_index, 3);
    assert_eq!(fetched.provenance.author_role, "user");

    // Instinct lifecycle
    let mut instinct = AtomicInstinct::new(
        "inst-sql",
        "write database migration",
        vec!["run_sql".into()],
    );
    assert_eq!(instinct.confidence, 0.5);

    // Reinforce
    instinct.reinforce();
    assert_eq!(instinct.confidence, 0.6);

    // Penalize
    instinct.penalize();
    assert_eq!(instinct.confidence, 0.55);

    store.register_instinct(instinct).await;
    let triggered = store
        .evaluate_instinct("Please write database migration for users table")
        .await;
    assert!(triggered.is_some());
    assert_eq!(triggered.unwrap().id, "inst-sql");
}
