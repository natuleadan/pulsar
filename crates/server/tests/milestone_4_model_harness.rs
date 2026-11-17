use pulsar_server::modules::providers::harness::{
    HarnessRegistry, ModelHarness, ModelHarnessDialect,
};

#[tokio::test]
async fn test_deepseek_r1_reasoning_and_tool_call() {
    let registry = HarnessRegistry::new();
    let prompt = registry
        .format_system_prompt("deepseek-r1", "You are an assistant.")
        .await;
    assert!(prompt.contains("<think>...</think>"));

    let raw_model_response = r#"<think>
Need to fetch the file contents to proceed.
I should call read_file.
</think>
Here is my plan, I will now inspect the repository.
```json
{
  "tool": "read_file",
  "arguments": {
    "path": "/workspace/README.md"
  }
}
```"#;

    let output = registry
        .parse_completion("deepseek-r1", raw_model_response)
        .await;

    assert_eq!(
        output.reasoning.as_deref(),
        Some("Need to fetch the file contents to proceed.\nI should call read_file.")
    );
    assert_eq!(output.tool_calls.len(), 1);
    assert_eq!(output.tool_calls[0].name, "read_file");
    assert_eq!(
        output.tool_calls[0]
            .arguments
            .get("path")
            .and_then(|v| v.as_str()),
        Some("/workspace/README.md")
    );
}

#[tokio::test]
async fn test_claude_xml_tool_call_parsing() {
    let registry = HarnessRegistry::new();
    let raw_xml = r#"I will execute the tool for you.
<invoke name="bash">
<parameter name="command">ls -la</parameter>
</invoke>"#;

    let output = registry.parse_completion("anthropic-claude", raw_xml).await;
    assert_eq!(output.tool_calls.len(), 1);
    assert_eq!(output.tool_calls[0].name, "bash");
    assert_eq!(
        output.tool_calls[0]
            .arguments
            .get("command")
            .and_then(|v| v.as_str()),
        Some("ls -la")
    );
}

#[tokio::test]
async fn test_openai_json_tool_call_parsing() {
    let registry = HarnessRegistry::new();
    let raw_json = r#"{
      "tool_calls": [
        {
          "function": {
            "name": "search_web",
            "arguments": "{\"query\":\"rust async\"}"
          }
        }
      ]
    }"#;

    let output = registry.parse_completion("openai-gpt", raw_json).await;
    assert_eq!(output.tool_calls.len(), 1);
    assert_eq!(output.tool_calls[0].name, "search_web");
    assert_eq!(
        output.tool_calls[0]
            .arguments
            .get("query")
            .and_then(|v| v.as_str()),
        Some("rust async")
    );
}

#[tokio::test]
async fn test_dynamic_harness_update_without_recompilation() {
    let registry = HarnessRegistry::new();
    let custom_harness = ModelHarness {
        id: "qwen-coder".to_string(),
        family: "qwen".to_string(),
        dialect: ModelHarnessDialect::GenericMarkdown,
        system_template: "QWEN_SPECIAL: {{base_prompt}}".to_string(),
        reasoning_open: None,
        reasoning_close: None,
        version: 2,
    };

    registry.register(custom_harness).await;

    let prompt = registry
        .format_system_prompt("qwen-coder", "Hello Qwen")
        .await;
    assert_eq!(prompt, "QWEN_SPECIAL: Hello Qwen");

    let loaded = registry.get("qwen-coder").await;
    assert!(loaded.is_some());
    assert_eq!(loaded.unwrap().version, 2);
}
