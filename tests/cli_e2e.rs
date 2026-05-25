//! End-to-end CLI integration tests. Drive the binary via the
//! captured-writers entry point so stdout/stderr are deterministic.

use lava::cli::{run_with_writers};

fn run(args: &[&str]) -> (i32, String, String) {
    let mut out: Vec<u8> = Vec::new();
    let mut err: Vec<u8> = Vec::new();
    let argv: Vec<std::ffi::OsString> = std::iter::once("lava".into())
        .chain(args.iter().map(|s| (*s).into()))
        .collect();
    let code = run_with_writers(argv, &mut out, &mut err);
    (
        code,
        String::from_utf8(out).unwrap_or_default(),
        String::from_utf8(err).unwrap_or_default(),
    )
}

fn tmpdir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lava-cli-e2e-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const VPC_TLISP: &str = r#"
(deflava-architecture demo-vpc
  :inputs ((:cidr "10.0.0.0/16"))
  :resources (
    (aws-vpc "demo"
      :cidr-block "{cidr}"
      :enable-dns-support #t)))
"#;

#[test]
fn ls_architectures_emits_every_bundled_entry() {
    let (code, out, _err) = run(&["ls", "architectures"]);
    assert_eq!(code, 0);
    assert!(out.contains("aws-vpc-network"));
    assert!(out.contains("cloudflare-dns-records"));
    assert!(out.contains("akeyless-secrets"));
}

#[test]
fn ls_interfaces_includes_doc_strings() {
    let (code, out, _err) = run(&["ls", "interfaces"]);
    assert_eq!(code, 0);
    // Each bundled architecture should now ship an interface (per
    // the deflava-interface header we authored).
    assert!(out.contains("aws-vpc-network"));
    assert!(out.contains("VPC")); // doc string
}

#[test]
fn show_interface_emits_json_with_typed_fields() {
    let (code, out, _err) = run(&["show", "interface", "aws-vpc-network"]);
    assert_eq!(code, 0);
    let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(parsed["name"], "aws-vpc-network");
    // The deflava-interface header has a :cidr input.
    assert!(parsed["inputs"]["cidr"].is_object());
}

#[test]
fn plan_renders_tlisp_file_to_terraform_json_on_stdout() {
    let dir = tmpdir();
    let path = dir.join("demo.tlisp");
    std::fs::write(&path, VPC_TLISP).unwrap();
    let (code, out, _err) = run(&["plan", path.to_str().unwrap()]);
    assert_eq!(code, 0);
    let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(parsed["resource"]["aws_vpc"]["demo"]["cidr_block"], "10.0.0.0/16");
}

#[test]
fn plan_threads_scalar_bindings() {
    let dir = tmpdir();
    let path = dir.join("demo.tlisp");
    std::fs::write(&path, VPC_TLISP).unwrap();
    let (code, out, _err) = run(&[
        "plan",
        path.to_str().unwrap(),
        "--binding",
        "cidr=172.31.0.0/16",
    ]);
    assert_eq!(code, 0);
    let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(parsed["resource"]["aws_vpc"]["demo"]["cidr_block"], "172.31.0.0/16");
}

#[test]
fn plan_writes_to_out_file_when_requested() {
    let dir = tmpdir();
    let path = dir.join("demo.tlisp");
    std::fs::write(&path, VPC_TLISP).unwrap();
    let out_path = dir.join("rendered.json");
    let (code, _out, _err) = run(&[
        "plan",
        path.to_str().unwrap(),
        "--out",
        out_path.to_str().unwrap(),
    ]);
    assert_eq!(code, 0);
    let body = std::fs::read_to_string(&out_path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["resource"]["aws_vpc"]["demo"]["cidr_block"], "10.0.0.0/16");
}

#[test]
fn plan_yaml_format_round_trips() {
    let dir = tmpdir();
    let path = dir.join("demo.tlisp");
    std::fs::write(&path, VPC_TLISP).unwrap();
    let (code, out, _err) = run(&[
        "plan",
        path.to_str().unwrap(),
        "--format",
        "yaml",
    ]);
    assert_eq!(code, 0);
    let parsed: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
    assert_eq!(
        parsed["resource"]["aws_vpc"]["demo"]["cidr_block"],
        serde_yaml::Value::String("10.0.0.0/16".into())
    );
}

#[test]
fn validate_with_passing_gate_exits_zero() {
    let dir = tmpdir();
    let path = dir.join("demo.tlisp");
    std::fs::write(&path, VPC_TLISP).unwrap();
    let (code, out, _err) = run(&[
        "validate",
        path.to_str().unwrap(),
        "--gate",
        "aws-vpc-network",
    ]);
    assert_eq!(code, 0);
    assert!(out.contains("validate ok"));
}

#[test]
fn validate_with_unknown_gate_exits_nonzero() {
    let dir = tmpdir();
    let path = dir.join("demo.tlisp");
    std::fs::write(&path, VPC_TLISP).unwrap();
    let (code, _out, err) = run(&[
        "validate",
        path.to_str().unwrap(),
        "--gate",
        "no-such-interface",
    ]);
    assert_ne!(code, 0);
    assert!(err.contains("validate failed"));
}

#[test]
fn no_args_emits_help_with_nonzero_exit() {
    let (code, _out, err) = run(&[]);
    assert_ne!(code, 0);
    assert!(err.contains("lava") || err.contains("Usage"));
}
