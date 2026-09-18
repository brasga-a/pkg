use assert_cmd::Command;
use serde_json::Value;
use tempfile::tempdir;

#[test]
fn mcp_handshake_and_profile_tool_use_json_rpc() {
    let temp = tempdir().unwrap();
    let input = concat!(
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\"}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"pkg.profile_create\",\"arguments\":{\"name\":\"agent\"}}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"pkg.list\",\"arguments\":{\"profile\":\"agent\"}}}\n",
    );
    let output = Command::cargo_bin("pkg")
        .unwrap()
        .args(["--data-dir", temp.path().to_str().unwrap(), "mcp"])
        .write_stdin(input)
        .output()
        .unwrap();
    assert!(output.status.success());
    let responses: Vec<Value> = output
        .stdout
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice(line).unwrap())
        .collect();
    assert_eq!(responses.len(), 3);
    assert_eq!(responses[0]["result"]["protocolVersion"], "2024-11-05");
    assert_eq!(
        responses[1]["result"]["structuredContent"]["status"],
        "success"
    );
    assert_eq!(
        responses[2]["result"]["structuredContent"],
        serde_json::json!([])
    );
}
