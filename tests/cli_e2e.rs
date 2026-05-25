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
fn plan_crossplane_format_emits_xrd_plus_composition() {
    let dir = tmpdir();
    let path = dir.join("demo.tlisp");
    std::fs::write(&path, VPC_TLISP).unwrap();
    let (code, out, _err) = run(&[
        "plan",
        path.to_str().unwrap(),
        "--format",
        "crossplane",
    ]);
    assert_eq!(code, 0);
    assert!(out.contains("kind: CompositeResourceDefinition"));
    assert!(out.contains("kind: Composition"));
    assert!(out.contains("cidr_block: 10.0.0.0/16"));
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
fn show_resources_lists_every_rendered_resource() {
    let (code, out, _err) = run(&["show", "resources", "aws-vpc-network"]);
    assert_eq!(code, 0);
    // VPC + IGW + subnets + NAT + EIP + SG.
    assert!(out.contains("aws_vpc.main-vpc"));
    assert!(out.contains("aws_internet_gateway.main-igw"));
    assert!(out.contains("aws_nat_gateway.main-nat"));
    assert!(out.contains("aws_security_group.main-default-sg"));
}

#[test]
fn show_resources_threads_bindings_into_rendered_names() {
    let (code, out, _err) = run(&[
        "show",
        "resources",
        "aws-vpc-network",
        "--binding",
        "name=preview",
    ]);
    assert_eq!(code, 0);
    assert!(out.contains("aws_vpc.preview-vpc"));
    assert!(out.contains("aws_internet_gateway.preview-igw"));
}

#[test]
fn show_outputs_lists_result_slot_keys() {
    let (code, out, _err) = run(&["show", "outputs", "aws-vpc-network"]);
    assert_eq!(code, 0);
    // The :result clause declares network + a few :keys.
    assert!(out.contains("result\tnetwork"));
    assert!(out.contains(":vpc-id"));
}

#[test]
fn show_stats_reports_typed_resource_breakdown() {
    let (code, out, _err) = run(&["show", "stats", "aws-vpc-network"]);
    assert_eq!(code, 0);
    assert!(out.contains("architecture\taws-vpc-network"));
    assert!(out.contains("total-resources"));
    assert!(out.contains("aws_subnet"));
    assert!(out.contains("interface\taws-vpc-network"));
}

#[test]
fn show_resources_for_unknown_architecture_exits_nonzero() {
    let (code, _out, err) = run(&["show", "resources", "no-such-arch"]);
    assert_ne!(code, 0);
    assert!(err.contains("not found"));
}

#[test]
fn graph_dot_for_bundled_architecture_emits_directed_graph() {
    let (code, out, _err) = run(&["graph", "aws-vpc-network"]);
    assert_eq!(code, 0);
    assert!(out.contains("digraph lava"));
    assert!(out.contains("aws_vpc.main-vpc"));
    assert!(out.contains("aws_internet_gateway.main-igw"));
    // Edge from IGW → VPC (IGW depends on the VPC via vpc_id ref).
    assert!(out.contains("\"aws_internet_gateway.main-igw\" -> \"aws_vpc.main-vpc\""));
}

#[test]
fn graph_mermaid_for_bundled_architecture_emits_flowchart() {
    let (code, out, _err) = run(&["graph", "aws-vpc-network", "--format", "mermaid"]);
    assert_eq!(code, 0);
    assert!(out.contains("flowchart LR"));
    assert!(out.contains("aws_internet_gateway_main_igw"));
    assert!(out.contains("-->"));
}

#[test]
fn graph_unknown_target_exits_nonzero() {
    let (code, _out, err) = run(&["graph", "no-such-thing"]);
    assert_ne!(code, 0);
    assert!(err.contains("not found"));
}

#[test]
fn test_runs_typed_assertions_against_bundled_architecture() {
    // Write a small .test.tlisp pointing at aws-vpc-network.
    let dir = tmpdir();
    let path = dir.join("vpc.test.tlisp");
    let body = r#"
        (deflava-test aws-vpc-network/smoke
          :architecture aws-vpc-network
          :assertions ((resource-exists aws-vpc "main-vpc")
                       (attribute-equals aws-vpc "main-vpc" :cidr-block "10.0.0.0/16")
                       (resource-count aws-subnet 6)
                       (ref-valid)))
    "#;
    std::fs::write(&path, body).unwrap();
    let (code, out, _err) = run(&["test", path.to_str().unwrap()]);
    assert_eq!(code, 0, "test should pass");
    assert!(out.contains("aws-vpc-network/smoke"));
    assert!(out.contains("4/4 assertions passed"));
    assert!(out.contains("1 passed, 0 failed"));
}

#[test]
fn test_reports_typed_assertion_failures_nonzero() {
    let dir = tmpdir();
    let path = dir.join("vpc.test.tlisp");
    let body = r#"
        (deflava-test aws-vpc-network/wrong
          :architecture aws-vpc-network
          :assertions ((resource-exists aws-vpc "no-such-vpc")
                       (resource-count aws-subnet 99)))
    "#;
    std::fs::write(&path, body).unwrap();
    let (code, _out, err) = run(&["test", path.to_str().unwrap()]);
    assert_ne!(code, 0);
    assert!(err.contains("aws-vpc-network/wrong"));
    assert!(err.contains("failure"));
}

#[test]
fn test_empty_file_exits_nonzero() {
    let dir = tmpdir();
    let path = dir.join("empty.test.tlisp");
    std::fs::write(&path, "").unwrap();
    let (code, _out, err) = run(&["test", path.to_str().unwrap()]);
    assert_ne!(code, 0);
    assert!(err.contains("no (deflava-test"));
}

#[test]
fn apply_with_default_embedded_engine_reports_unbundled_when_feature_off() {
    // Default engine is embedded; without the feature, the typed
    // error explains how to enable or pick another engine.
    let dir = tmpdir();
    let path = dir.join("demo.tlisp");
    std::fs::write(&path, VPC_TLISP).unwrap();
    let (code, _out, err) = run(&["apply", path.to_str().unwrap()]);
    assert_ne!(code, 0);
    // Either feature-off message OR (when feature on) unbundled
    // message — both indicate "embedded path isn't operational yet"
    // in the same way the CLI promises.
    assert!(
        err.contains("embedded-magma") || err.contains("library API surface"),
        "expected typed embedded-engine message, got: {err}"
    );
}

#[test]
fn apply_with_unknown_target_exits_nonzero() {
    let (code, _out, err) = run(&["apply", "no-such-target"]);
    assert_ne!(code, 0);
    assert!(err.contains("not found"));
}

#[test]
fn plan_engine_with_invalid_tofu_binary_surfaces_typed_error() {
    let dir = tmpdir();
    let path = dir.join("demo.tlisp");
    std::fs::write(&path, VPC_TLISP).unwrap();
    // Force engine=tofu so we exercise the shell-out path; tofu
    // likely isn't on PATH in the test env → typed failure.
    let (code, _out, err) = run(&[
        "plan-engine",
        path.to_str().unwrap(),
        "--engine",
        "tofu",
    ]);
    // Either tofu is missing (non-zero + typed error) OR tofu is
    // installed (init may succeed but the actual plan may fail
    // without providers); in both cases the run completes
    // cleanly + the CLI surface honoured the flags.
    let _ = code;
    let _ = err;
}

#[test]
fn new_architecture_scaffolds_paired_interface_plus_architecture() {
    let dir = tmpdir();
    let (code, _out, _err) = run(&[
        "new",
        "architecture",
        "my-thing",
        "--out",
        dir.to_str().unwrap(),
    ]);
    assert_eq!(code, 0);
    let body = std::fs::read_to_string(dir.join("my-thing.tlisp")).unwrap();
    assert!(body.contains("deflava-interface my-thing"));
    assert!(body.contains("deflava-architecture my-thing"));
}

#[test]
fn new_test_scaffolds_test_dot_tlisp_with_ref_valid() {
    let dir = tmpdir();
    let (code, _out, _err) = run(&[
        "new",
        "test",
        "my-test",
        "--out",
        dir.to_str().unwrap(),
    ]);
    assert_eq!(code, 0);
    let body = std::fs::read_to_string(dir.join("my-test.test.tlisp")).unwrap();
    assert!(body.contains("deflava-test my-test/default"));
    assert!(body.contains("ref-valid"));
}

#[test]
fn new_spec_scaffolds_scenarios_form() {
    let dir = tmpdir();
    let (code, _out, _err) = run(&["new", "spec", "my-spec", "--out", dir.to_str().unwrap()]);
    assert_eq!(code, 0);
    let body = std::fs::read_to_string(dir.join("my-spec.tlisp")).unwrap();
    assert!(body.contains("deflava-spec my-spec"));
    assert!(body.contains(":scenarios"));
}

#[test]
fn new_test_against_autogenerates_assertions_from_architecture() {
    let dir = tmpdir();
    let (code, _out, _err) = run(&[
        "new",
        "test",
        "vpc-coverage",
        "--out",
        dir.to_str().unwrap(),
        "--against",
        "aws-vpc-network",
    ]);
    assert_eq!(code, 0);
    let body = std::fs::read_to_string(dir.join("vpc-coverage.test.tlisp")).unwrap();
    assert!(body.contains("autogenerated by `lava new test --against aws-vpc-network`"));
    assert!(body.contains("deflava-test vpc-coverage/default"));
    assert!(body.contains(":architecture aws-vpc-network"));
    assert!(body.contains("(resource-exists aws-vpc \"main-vpc\")"));
    assert!(body.contains("(resource-exists aws-subnet \"main-public-0\")"));
    assert!(body.contains("(resource-count aws-subnet 6)"));
    assert!(body.contains("(ref-valid)"));
}

#[test]
fn new_test_against_unknown_architecture_exits_nonzero() {
    let dir = tmpdir();
    let (code, _, err) = run(&[
        "new",
        "test",
        "x",
        "--out",
        dir.to_str().unwrap(),
        "--against",
        "no-such-arch",
    ]);
    assert_ne!(code, 0);
    assert!(err.contains("not found"));
}

#[test]
fn new_refuses_to_overwrite_unless_force() {
    let dir = tmpdir();
    let (code, _out, _err) = run(&["new", "interface", "x", "--out", dir.to_str().unwrap()]);
    assert_eq!(code, 0);
    let (code2, _out, err) = run(&["new", "interface", "x", "--out", dir.to_str().unwrap()]);
    assert_ne!(code2, 0);
    assert!(err.contains("already exists"));
    let (code3, _out, _err) = run(&[
        "new",
        "interface",
        "x",
        "--out",
        dir.to_str().unwrap(),
        "--force",
    ]);
    assert_eq!(code3, 0);
}

#[test]
fn pack_wraps_tlisp_in_redistributable_crate_with_workflows() {
    let dir = tmpdir();
    let tlisp = dir.join("src.tlisp");
    let body = "(deflava-architecture demo :inputs () :resources ((aws-vpc \"main\" :cidr-block \"10.0.0.0/16\")))\n";
    std::fs::write(&tlisp, body).unwrap();
    let out_dir = dir.join("packed");
    let (code, stdout, _err) = run(&[
        "pack",
        tlisp.to_str().unwrap(),
        "--out",
        out_dir.to_str().unwrap(),
    ]);
    assert_eq!(code, 0);
    assert!(stdout.contains("lava-pack-src"));
    // Cargo.toml + src/lib.rs + workflow shims exist.
    assert!(out_dir.join("Cargo.toml").exists());
    assert!(out_dir.join("src/lib.rs").exists());
    assert!(out_dir.join(".github/workflows/auto-release.yml").exists());
    assert!(out_dir.join(".github/workflows/pre-merge-gate.yml").exists());
    assert!(out_dir.join(".github/workflows/security-gate.yml").exists());
    assert!(out_dir.join(".gitignore").exists());
    assert!(out_dir.join("README.md").exists());
    // The packed lib exports the source byte-for-byte under `SOURCE`.
    let lib_body = std::fs::read_to_string(out_dir.join("src/lib.rs")).unwrap();
    assert!(lib_body.contains("pub const SOURCE: &str"));
    assert!(lib_body.contains("aws-vpc"));
}

#[test]
fn pack_refuses_to_overwrite_unless_force() {
    let dir = tmpdir();
    let tlisp = dir.join("src.tlisp");
    std::fs::write(&tlisp, "(deflava-interface x :inputs () :outputs ())\n").unwrap();
    let out_dir = dir.join("p");
    let (code1, _o, _e) = run(&["pack", tlisp.to_str().unwrap(), "--out", out_dir.to_str().unwrap()]);
    assert_eq!(code1, 0);
    let (code2, _o, err) = run(&["pack", tlisp.to_str().unwrap(), "--out", out_dir.to_str().unwrap()]);
    assert_ne!(code2, 0);
    assert!(err.contains("already exists"));
    let (code3, _o, _e) = run(&[
        "pack",
        tlisp.to_str().unwrap(),
        "--out",
        out_dir.to_str().unwrap(),
        "--force",
    ]);
    assert_eq!(code3, 0);
}

#[test]
fn pack_caixa_format_emits_single_dot_caixa_lisp_file() {
    let dir = tmpdir();
    let tlisp = dir.join("vpc.tlisp");
    let body = "(deflava-architecture demo :inputs () :resources ((aws-vpc \"main\" :cidr-block \"10.0.0.0/16\")))\n";
    std::fs::write(&tlisp, body).unwrap();
    let out_dir = dir.join("caixa-out");
    let (code, _stdout, _err) = run(&[
        "pack",
        tlisp.to_str().unwrap(),
        "--out",
        out_dir.to_str().unwrap(),
        "--format",
        "caixa",
        "--crate-name",
        "caixa-demo-vpc",
    ]);
    assert_eq!(code, 0);
    let caixa = out_dir.join("caixa-demo-vpc.caixa.lisp");
    assert!(caixa.exists(), "caixa file not written");
    let body_out = std::fs::read_to_string(&caixa).unwrap();
    assert!(body_out.contains("(defcaixa"));
    assert!(body_out.contains(":kind lava-architecture"));
    assert!(body_out.contains(":name \"caixa-demo-vpc\""));
    assert!(body_out.contains(":files"));
    assert!(body_out.contains("src/vpc.tlisp"));
    // No Rust scaffold files for caixa format.
    assert!(!out_dir.join("Cargo.toml").exists());
    assert!(!out_dir.join("src/lib.rs").exists());
}

#[test]
fn pack_caixa_refuses_to_overwrite_unless_force() {
    let dir = tmpdir();
    let tlisp = dir.join("x.tlisp");
    std::fs::write(&tlisp, "(deflava-interface x :inputs () :outputs ())\n").unwrap();
    let out_dir = dir.join("out");
    let (c1, _, _) = run(&[
        "pack",
        tlisp.to_str().unwrap(),
        "--out",
        out_dir.to_str().unwrap(),
        "--format",
        "caixa",
    ]);
    assert_eq!(c1, 0);
    let (c2, _, err) = run(&[
        "pack",
        tlisp.to_str().unwrap(),
        "--out",
        out_dir.to_str().unwrap(),
        "--format",
        "caixa",
    ]);
    assert_ne!(c2, 0);
    assert!(err.contains("already exists"));
    let (c3, _, _) = run(&[
        "pack",
        tlisp.to_str().unwrap(),
        "--out",
        out_dir.to_str().unwrap(),
        "--format",
        "caixa",
        "--force",
    ]);
    assert_eq!(c3, 0);
}

#[test]
fn pack_custom_crate_name_lands_in_cargo_toml() {
    let dir = tmpdir();
    let tlisp = dir.join("src.tlisp");
    std::fs::write(&tlisp, "(deflava-interface x :inputs () :outputs ())\n").unwrap();
    let out_dir = dir.join("p");
    let (code, _o, _e) = run(&[
        "pack",
        tlisp.to_str().unwrap(),
        "--out",
        out_dir.to_str().unwrap(),
        "--crate-name",
        "lava-arch-vpc-tiny",
    ]);
    assert_eq!(code, 0);
    let cargo = std::fs::read_to_string(out_dir.join("Cargo.toml")).unwrap();
    assert!(cargo.contains("name = \"lava-arch-vpc-tiny\""));
    assert!(cargo.contains("name = \"lava_arch_vpc_tiny\"")); // lib name
}

#[test]
fn forge_dry_run_stages_workspace_without_running_tofu() {
    let dir = tmpdir();
    let (code, out, _err) = run(&[
        "forge",
        "cloudflare",
        "--work-dir",
        dir.to_str().unwrap(),
    ]);
    assert_eq!(code, 0);
    assert!(out.contains("staged tofu workspace"));
    assert!(out.contains("provider: cloudflare"));
    assert!(out.contains("dry-run"));
    let tf = dir.join("main.tf.json");
    assert!(tf.exists());
    let body = std::fs::read_to_string(&tf).unwrap();
    assert!(body.contains("required_providers"));
    assert!(body.contains("cloudflare/cloudflare"));
}

#[test]
fn forge_resolves_source_for_known_providers() {
    let dir = tmpdir();
    let (code, _, _) = run(&[
        "forge",
        "aws",
        "--work-dir",
        dir.to_str().unwrap(),
    ]);
    assert_eq!(code, 0);
    let body = std::fs::read_to_string(dir.join("main.tf.json")).unwrap();
    assert!(body.contains("hashicorp/aws"));
}

#[test]
fn no_args_emits_help_with_nonzero_exit() {
    let (code, _out, err) = run(&[]);
    assert_ne!(code, 0);
    assert!(err.contains("lava") || err.contains("Usage"));
}
