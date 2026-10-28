use mcp_core::{
    adapters, catalog,
    document::ConfigDoc,
    interpolate::VarCtx,
    plan::{plan_file, MergeMode},
    resolve, McpEntry, Platform, Scope,
};
use std::collections::BTreeMap;

#[test]
fn resolved_environment_values_are_written_but_never_displayed() {
    let mut entry = McpEntry::new_stdio(
        "local",
        "python",
        vec!["--key".into(), "${ENV:API_KEY}".into()],
    );
    entry.env.insert("API_KEY".into(), "${ENV:API_KEY}".into());
    entry
        .headers
        .insert("Authorization".into(), "Bearer ${ENV:API_KEY}".into());
    let ctx = VarCtx {
        home: None,
        project_root: None,
        vars: BTreeMap::from([("API_KEY".into(), "your_api_key_here".into())]),
        read_process_env: false,
    };
    let (resolved, missing) = resolve::resolve(&entry, Platform::Windows, &ctx);
    assert!(missing.is_empty());
    let plan = plan_file(
        adapters::by_id("claude-code").unwrap(),
        "test.json",
        Scope::User,
        None,
        &[resolved],
        MergeMode::Merge,
        false,
    )
    .unwrap();
    assert!(plan.new_content.contains("your_api_key_here"));
    assert!(!plan.diff_text().contains("your_api_key_here"));
    assert!(plan
        .line_diff
        .iter()
        .all(|line| !line.text.contains("your_api_key_here")));
}

#[test]
fn parser_errors_do_not_echo_configuration_source() {
    let error = ConfigDoc::parse(
        adapters::Format::Toml,
        "[mcp_servers.private]\npassword=your_secret_here\n",
    )
    .err()
    .unwrap()
    .to_string();
    assert!(!error.contains("your_secret_here"));
    let error = catalog::parse("version: 1\nentries: [{name: a, transport: your_secret_here}]\n")
        .unwrap_err()
        .to_string();
    assert!(!error.contains("your_secret_here"));
    assert!(error.contains("清单解析失败"));
}
