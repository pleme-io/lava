//! Typed argv parsing + subcommand dispatch for the `lava` CLI.
//!
//! Keeps `main.rs` minimal; tests call `run_with_writer` directly with
//! captured stdout for fixtures.

use clap::{Parser, Subcommand, ValueEnum};
use indexmap::IndexMap;
use lava_architectures::{interface_for, BUNDLED_ARCHITECTURES};
use magma_lava::{synthesize, LavaPlanArgs};
use std::io::Write;

#[derive(Parser, Debug)]
#[command(
    name = "lava",
    version,
    about = "lava — operator CLI for the lava IaC suite (.tlisp → magma terraform.json)"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Render a .tlisp file → terraform.json (optionally schema-gated).
    Plan(PlanArgs),
    /// Render a bundled architecture by name → terraform.json.
    Render(RenderArgs),
    /// Run the schema gate against the supplied bindings + .tlisp,
    /// without rendering. Exits 0 if the bag validates, non-zero on
    /// typed mismatch.
    Validate(ValidateArgs),
    /// Catalog inspection.
    Ls {
        #[command(subcommand)]
        what: LsTarget,
    },
    /// Show the typed Interface registered for a bundled architecture.
    Show {
        #[command(subcommand)]
        what: ShowTarget,
    },
    /// Emit a dependency graph (DOT or Mermaid) for the architecture.
    Graph(GraphArgs),
    /// Run typed assertion tests authored in tatara-lisp.
    Test(TestArgs),
}

#[derive(Parser, Debug)]
pub struct TestArgs {
    /// Path to a .test.tlisp file. May contain multiple
    /// (deflava-test …) forms. Each forms a TestCase.
    pub path: std::path::PathBuf,
    /// Override the architecture for every case in the file. Useful
    /// when the .test.tlisp doesn't pin one + the operator wants to
    /// re-target the suite at a different bundled architecture.
    #[arg(long, value_name = "NAME")]
    pub architecture: Option<String>,
    /// `key=value` bindings layered on top of each case's own bindings.
    #[arg(long = "binding", value_name = "KEY=VALUE")]
    pub bindings: Vec<String>,
}

#[derive(Parser, Debug)]
pub struct GraphArgs {
    /// Path to a .tlisp file OR a bundled architecture name.
    pub target: String,
    #[arg(long = "binding", value_name = "KEY=VALUE")]
    pub bindings: Vec<String>,
    #[arg(long = "binding-list", value_name = "KEY=VAL,VAL,...")]
    pub list_bindings: Vec<String>,
    #[arg(long, default_value_t = GraphFormat::Dot)]
    pub format: GraphFormat,
    #[arg(long, value_name = "FILE")]
    pub out: Option<std::path::PathBuf>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum GraphFormat {
    /// Graphviz DOT — `dot -Tpng …` or `xdot`.
    Dot,
    /// Mermaid flowchart — drops into markdown.
    Mermaid,
}

impl std::fmt::Display for GraphFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dot => f.write_str("dot"),
            Self::Mermaid => f.write_str("mermaid"),
        }
    }
}

#[derive(Parser, Debug)]
pub struct PlanArgs {
    /// Path to the .tlisp source file.
    pub path: std::path::PathBuf,
    /// Optional schema gate (bundled interface name).
    #[arg(long, value_name = "INTERFACE")]
    pub gate: Option<String>,
    /// `key=value` bindings (repeatable).
    #[arg(long = "binding", value_name = "KEY=VALUE")]
    pub bindings: Vec<String>,
    /// `key=v1,v2,...` list bindings (repeatable).
    #[arg(long = "binding-list", value_name = "KEY=VAL,VAL,...")]
    pub list_bindings: Vec<String>,
    /// Write the rendered output to a file (otherwise stdout).
    #[arg(long, value_name = "FILE")]
    pub out: Option<std::path::PathBuf>,
    /// Output format.
    #[arg(long, default_value_t = OutFormat::Json)]
    pub format: OutFormat,
}

#[derive(Parser, Debug)]
pub struct RenderArgs {
    /// Architecture name (must appear in `lava ls architectures`).
    pub name: String,
    #[arg(long = "binding", value_name = "KEY=VALUE")]
    pub bindings: Vec<String>,
    #[arg(long = "binding-list", value_name = "KEY=VAL,VAL,...")]
    pub list_bindings: Vec<String>,
    #[arg(long, value_name = "FILE")]
    pub out: Option<std::path::PathBuf>,
    #[arg(long, default_value_t = OutFormat::Json)]
    pub format: OutFormat,
}

#[derive(Parser, Debug)]
pub struct ValidateArgs {
    pub path: std::path::PathBuf,
    #[arg(long, value_name = "INTERFACE")]
    pub gate: String,
    #[arg(long = "binding", value_name = "KEY=VALUE")]
    pub bindings: Vec<String>,
    #[arg(long = "binding-list", value_name = "KEY=VAL,VAL,...")]
    pub list_bindings: Vec<String>,
}

#[derive(Subcommand, Debug)]
pub enum LsTarget {
    /// List every bundled architecture name + expected resource count.
    Architectures,
    /// List every typed interface (one per bundled architecture).
    Interfaces,
}

#[derive(Subcommand, Debug)]
pub enum ShowTarget {
    /// Print the typed Interface for one architecture (JSON).
    Interface { name: String },
    /// List every resource the architecture renders: `<type_id>.<name>`.
    Resources {
        name: String,
        /// Optional bindings (so resource names that interpolate
        /// `{name}` show the resolved value).
        #[arg(long = "binding", value_name = "KEY=VALUE")]
        bindings: Vec<String>,
    },
    /// Show the architecture's declared output slot (the `:result`
    /// clause): which keys downstream stacks can read.
    Outputs { name: String },
    /// Quick stats — resource counts grouped by type, plus the
    /// architecture's declared interface name (if any).
    Stats { name: String },
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum OutFormat {
    /// terraform.json (magma-compatible).
    Json,
    /// terraform.json shape serialized to YAML.
    Yaml,
    /// Crossplane Composition + CompositeResourceDefinition pair.
    Crossplane,
}

impl std::fmt::Display for OutFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json => f.write_str("json"),
            Self::Yaml => f.write_str("yaml"),
            Self::Crossplane => f.write_str("crossplane"),
        }
    }
}

/// Entry point — parses argv, dispatches, returns process exit code.
#[must_use]
pub fn run<I>(args: I) -> i32
where
    I: IntoIterator,
    I::Item: Into<std::ffi::OsString> + Clone,
{
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let stderr = std::io::stderr();
    let mut err = stderr.lock();
    run_with_writers(args, &mut out, &mut err)
}

/// Same as [`run`] but writes to the supplied writers — used by the
/// integration tests to capture output.
pub fn run_with_writers<I>(args: I, out: &mut dyn Write, err: &mut dyn Write) -> i32
where
    I: IntoIterator,
    I::Item: Into<std::ffi::OsString> + Clone,
{
    let cli = match Cli::try_parse_from(args) {
        Ok(c) => c,
        Err(e) => {
            // Clap exit codes: 0 for --help / --version, 2 otherwise.
            let _ = writeln!(err, "{e}");
            return if e.exit_code() == 0 { 0 } else { 2 };
        }
    };
    match cli.command {
        Command::Plan(args) => cmd_plan(&args, out, err),
        Command::Render(args) => cmd_render(&args, out, err),
        Command::Validate(args) => cmd_validate(&args, out, err),
        Command::Ls { what } => cmd_ls(&what, out, err),
        Command::Show { what } => cmd_show(&what, out, err),
        Command::Graph(args) => cmd_graph(&args, out, err),
        Command::Test(args) => cmd_test(&args, out, err),
    }
}

fn cmd_test(args: &TestArgs, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let src = match std::fs::read_to_string(&args.path) {
        Ok(s) => s,
        Err(e) => {
            let _ = writeln!(err, "lava test: read {}: {e}", args.path.display());
            return 1;
        }
    };
    let cases = match lava_test::tests_in_source(&src) {
        Ok(cs) => cs,
        Err(e) => {
            let _ = writeln!(err, "lava test: parse: {e}");
            return 1;
        }
    };
    if cases.is_empty() {
        let _ = writeln!(err, "lava test: no (deflava-test …) forms in {}", args.path.display());
        return 1;
    }

    let cli_bindings = parse_kv(&args.bindings);
    let mut total_pass: usize = 0;
    let mut total_fail: usize = 0;

    for case in &cases {
        let arch_name = args
            .architecture
            .clone()
            .or_else(|| case.architecture.clone());
        let Some(arch_name) = arch_name else {
            let _ = writeln!(
                err,
                "  ✗ {} — no :architecture set (use --architecture)",
                case.name
            );
            total_fail += 1;
            continue;
        };
        let plan = match render_case(&arch_name, &case.bindings, &cli_bindings, err) {
            Some(p) => p,
            None => {
                total_fail += 1;
                continue;
            }
        };
        let ctx = match lava_test::AssertContext::from_architecture(&plan.architecture) {
            Ok(c) => c,
            Err(e) => {
                let _ = writeln!(err, "  ✗ {} — render: {e}", case.name);
                total_fail += 1;
                continue;
            }
        };
        let outcome = lava_test::run_case_against(case, &ctx);
        if outcome.ok() {
            let _ = writeln!(
                out,
                "  ✓ {} — {}/{} assertions passed",
                outcome.name,
                outcome.passed,
                outcome.passed + outcome.failures.len()
            );
            total_pass += 1;
        } else {
            let _ = writeln!(
                err,
                "  ✗ {} — {} failure(s):",
                outcome.name,
                outcome.failures.len()
            );
            for f in &outcome.failures {
                let pointer = f.pointer.as_deref().unwrap_or("-");
                let _ = writeln!(err, "      • {} [{}]: {}", f.assertion, pointer, f.message);
            }
            total_fail += 1;
        }
    }

    let _ = writeln!(out, "lava test: {total_pass} passed, {total_fail} failed");
    if total_fail == 0 {
        0
    } else {
        1
    }
}

fn render_case(
    arch_name: &str,
    case_bindings: &indexmap::IndexMap<String, String>,
    cli_bindings: &indexmap::IndexMap<String, String>,
    err: &mut dyn Write,
) -> Option<magma_lava::LavaPlan> {
    let path = bundled_source_path(arch_name);
    if !path.exists() {
        let _ = writeln!(
            err,
            "  ✗ {arch_name} — bundled architecture not found at {}",
            path.display()
        );
        return None;
    }
    let mut bindings: IndexMap<String, magma_lava::Binding> = IndexMap::new();
    for (k, v) in case_bindings {
        bindings.insert(k.clone(), magma_lava::Binding::Scalar(v.clone()));
    }
    for (k, v) in cli_bindings {
        bindings.insert(k.clone(), magma_lava::Binding::Scalar(v.clone()));
    }
    let plan_args = LavaPlanArgs {
        path,
        bindings,
        gate_with: None,
        runtime_kind: None,
    };
    match synthesize(&plan_args) {
        Ok(p) => Some(p),
        Err(e) => {
            let _ = writeln!(err, "  ✗ {arch_name} — render: {e}");
            None
        }
    }
}

fn parse_kv(items: &[String]) -> indexmap::IndexMap<String, String> {
    let mut out: indexmap::IndexMap<String, String> = indexmap::IndexMap::new();
    for kv in items {
        if let Some((k, v)) = kv.split_once('=') {
            out.insert(k.to_string(), v.to_string());
        }
    }
    out
}

fn cmd_graph(args: &GraphArgs, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    // `target` resolves either to a bundled architecture name or to a
    // .tlisp file path on disk. Try bundled first; fall back to path.
    let path = {
        let bundled = bundled_source_path(&args.target);
        if bundled.exists() {
            bundled
        } else {
            std::path::PathBuf::from(&args.target)
        }
    };
    if !path.exists() {
        let _ = writeln!(
            err,
            "lava graph: target `{}` not found (tried bundled and direct path)",
            args.target
        );
        return 1;
    }
    let plan_args = LavaPlanArgs {
        path,
        bindings: parse_bindings(&args.bindings, &args.list_bindings),
        gate_with: None,
        runtime_kind: None,
    };
    let plan = match synthesize(&plan_args) {
        Ok(p) => p,
        Err(e) => {
            let _ = writeln!(err, "lava graph render failed: {e}");
            return 1;
        }
    };
    let serialized = match args.format {
        GraphFormat::Dot => render_dot_graph(&plan.architecture),
        GraphFormat::Mermaid => render_mermaid_graph(&plan.architecture),
    };
    match &args.out {
        Some(path) => match std::fs::write(path, &serialized) {
            Ok(()) => {
                let _ = writeln!(out, "wrote {} bytes → {}", serialized.len(), path.display());
                0
            }
            Err(e) => {
                let _ = writeln!(err, "lava graph write failed: {e}");
                1
            }
        },
        None => {
            let _ = out.write_all(serialized.as_bytes());
            let _ = out.write_all(b"\n");
            0
        }
    }
}

// ── Typed graph IR ──────────────────────────────────────────────────
//
// Per ★★ TYPED EMISSION: we don't format!() DOT / Mermaid syntax.
// Build a typed Graph value, then route it through one of two
// Display impls (DotGraph / MermaidGraph). Adding a new graph
// format = one more wrapper + one more Display impl, never any
// scattered format!() calls.

#[derive(Debug, Clone)]
struct GraphNode {
    type_id: String,
    name: String,
}

#[derive(Debug, Clone)]
struct GraphEdge {
    from: GraphNode,
    to: GraphNode,
}

#[derive(Debug, Default, Clone)]
struct Graph {
    nodes: Vec<GraphNode>,
    edges: Vec<GraphEdge>,
}

impl Graph {
    fn from_architecture(arch: &lava_architectures::Architecture) -> Self {
        let mut g = Self::default();
        for r in &arch.resources {
            let from = GraphNode {
                type_id: r.type_id.clone(),
                name: r.name.clone(),
            };
            g.nodes.push(from.clone());
            for (_, v) in &r.attributes {
                for dep in collect_refs(v) {
                    g.edges.push(GraphEdge {
                        from: from.clone(),
                        to: GraphNode {
                            type_id: dep.type_id,
                            name: dep.name,
                        },
                    });
                }
            }
        }
        g
    }
}

struct DotGraph<'a>(&'a Graph);
struct MermaidGraph<'a>(&'a Graph);

impl std::fmt::Display for DotGraph<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "digraph lava {{")?;
        writeln!(f, "  rankdir=LR;")?;
        writeln!(f, "  node [shape=box, style=rounded, fontname=\"monospace\"];")?;
        for n in &self.0.nodes {
            writeln!(f, "  \"{}.{}\";", n.type_id, n.name)?;
        }
        for e in &self.0.edges {
            writeln!(
                f,
                "  \"{}.{}\" -> \"{}.{}\";",
                e.from.type_id, e.from.name, e.to.type_id, e.to.name
            )?;
        }
        writeln!(f, "}}")
    }
}

impl std::fmt::Display for MermaidGraph<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "flowchart LR")?;
        // Collect uniques via BTreeSet for deterministic order.
        let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for n in self.0.nodes.iter().chain(self.0.edges.iter().flat_map(|e| {
            std::iter::once(&e.from).chain(std::iter::once(&e.to))
        })) {
            let id = mermaid_id(&n.type_id, &n.name);
            if seen.insert(id.clone()) {
                writeln!(f, "  {id}[\"{}.{}\"]", n.type_id, n.name)?;
            }
        }
        for e in &self.0.edges {
            writeln!(
                f,
                "  {} --> {}",
                mermaid_id(&e.from.type_id, &e.from.name),
                mermaid_id(&e.to.type_id, &e.to.name)
            )?;
        }
        Ok(())
    }
}

fn mermaid_id(type_id: &str, name: &str) -> String {
    let mut out = String::with_capacity(type_id.len() + 1 + name.len());
    for c in type_id.chars().chain(std::iter::once('_')).chain(name.chars()) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    out
}

fn render_dot_graph(arch: &lava_architectures::Architecture) -> String {
    let g = Graph::from_architecture(arch);
    DotGraph(&g).to_string()
}

fn render_mermaid_graph(arch: &lava_architectures::Architecture) -> String {
    let g = Graph::from_architecture(arch);
    MermaidGraph(&g).to_string()
}

fn collect_refs(v: &lava_architectures::Value) -> Vec<lava_architectures::ResourceRef> {
    let mut out = Vec::new();
    walk_refs(v, &mut out);
    out
}

fn walk_refs(v: &lava_architectures::Value, out: &mut Vec<lava_architectures::ResourceRef>) {
    use lava_architectures::Value;
    match v {
        Value::Ref(r) => out.push(r.clone()),
        Value::Json(json) => walk_json(json, out),
    }
}

fn walk_json(v: &serde_json::Value, out: &mut Vec<lava_architectures::ResourceRef>) {
    match v {
        serde_json::Value::Array(items) => {
            for item in items {
                walk_json(item, out);
            }
        }
        serde_json::Value::Object(map) => {
            for (_, val) in map {
                walk_json(val, out);
            }
        }
        _ => {}
    }
}

fn cmd_plan(args: &PlanArgs, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let bindings = parse_bindings(&args.bindings, &args.list_bindings);
    let plan_args = LavaPlanArgs {
        path: args.path.clone(),
        bindings,
        gate_with: args.gate.clone(),
        runtime_kind: None,
    };
    match synthesize(&plan_args) {
        Ok(plan) => emit_plan(&plan, args.format, args.out.as_ref(), out, err),
        Err(e) => {
            let _ = writeln!(err, "lava plan failed: {e}");
            1
        }
    }
}

fn cmd_render(args: &RenderArgs, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    // Render = plan against the bundled architecture's source path.
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|p| p.join("lava-architectures").join("architectures").join(format!("{}.tlisp", args.name)))
        .unwrap_or_else(|| std::path::PathBuf::from(format!("{}.tlisp", args.name)));
    if !path.exists() {
        let _ = writeln!(
            err,
            "lava render: bundled architecture `{}` not found at {}",
            args.name,
            path.display()
        );
        return 1;
    }
    let plan_args = LavaPlanArgs {
        path,
        bindings: parse_bindings(&args.bindings, &args.list_bindings),
        gate_with: None,
        runtime_kind: None,
    };
    match synthesize(&plan_args) {
        Ok(plan) => emit_plan(&plan, args.format, args.out.as_ref(), out, err),
        Err(e) => {
            let _ = writeln!(err, "lava render failed: {e}");
            1
        }
    }
}

fn cmd_validate(args: &ValidateArgs, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let plan_args = LavaPlanArgs {
        path: args.path.clone(),
        bindings: parse_bindings(&args.bindings, &args.list_bindings),
        gate_with: Some(args.gate.clone()),
        runtime_kind: None,
    };
    // synthesize runs the schema gate as a side-effect of evaluation;
    // we don't need to write the JSON, just report green.
    match synthesize(&plan_args) {
        Ok(plan) => {
            let _ = writeln!(
                out,
                "validate ok — {} resources, runtime={}",
                count_resources(&plan.terraform_json),
                plan.runtime_kind
            );
            0
        }
        Err(e) => {
            let _ = writeln!(err, "validate failed: {e}");
            1
        }
    }
}

fn cmd_ls(target: &LsTarget, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    match target {
        LsTarget::Architectures => {
            for (name, min_resources) in BUNDLED_ARCHITECTURES {
                let _ = writeln!(out, "{name}\t(>= {min_resources} resources)");
            }
            0
        }
        LsTarget::Interfaces => {
            for (name, _) in BUNDLED_ARCHITECTURES {
                if let Some(iface) = interface_for(name) {
                    let _ = writeln!(
                        out,
                        "{name}\t{}",
                        iface.doc.as_deref().unwrap_or("(no doc)")
                    );
                } else {
                    let _ = writeln!(err, "{name}\t(no interface registered)");
                }
            }
            0
        }
    }
}

fn cmd_show(target: &ShowTarget, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    match target {
        ShowTarget::Interface { name } => match interface_for(name) {
            Some(iface) => {
                let json = serde_json::to_string_pretty(&iface).unwrap_or_default();
                let _ = writeln!(out, "{json}");
                0
            }
            None => {
                let _ = writeln!(err, "lava show interface: no interface registered for `{name}`");
                1
            }
        },
        ShowTarget::Resources { name, bindings } => {
            let Some(plan) = render_bundled(name, bindings, &[], err) else {
                return 1;
            };
            let mut rows: Vec<(String, String)> = Vec::new();
            if let Some(by_type) = plan.terraform_json["resource"].as_object() {
                for (type_id, by_name) in by_type {
                    if let Some(by_name_map) = by_name.as_object() {
                        for n in by_name_map.keys() {
                            rows.push((type_id.clone(), n.clone()));
                        }
                    }
                }
            }
            rows.sort();
            for (type_id, n) in rows {
                let _ = writeln!(out, "{type_id}.{n}");
            }
            0
        }
        ShowTarget::Outputs { name } => {
            // Outputs live in the architecture's :result clause —
            // re-read the .tlisp source and pull them out by inspecting
            // the parsed s-expressions. We don't need to evaluate the
            // architecture, just look at the :result form.
            let src_path = bundled_source_path(name);
            let src = match std::fs::read_to_string(&src_path) {
                Ok(s) => s,
                Err(e) => {
                    let _ = writeln!(
                        err,
                        "lava show outputs: can't read {}: {e}",
                        src_path.display()
                    );
                    return 1;
                }
            };
            let forms = match lava_eval::parse_all(&src) {
                Ok(f) => f,
                Err(e) => {
                    let _ = writeln!(err, "lava show outputs: parse: {e}");
                    return 1;
                }
            };
            for form in &forms {
                if let Some(xs) = form.as_list() {
                    if xs.first().and_then(lava_eval::Sx::as_sym)
                        == Some("deflava-architecture")
                    {
                        emit_result_slot(xs, out);
                        return 0;
                    }
                }
            }
            let _ = writeln!(err, "lava show outputs: no deflava-architecture form found");
            1
        }
        ShowTarget::Stats { name } => {
            let Some(plan) = render_bundled(name, &[], &[], err) else {
                return 1;
            };
            let mut counts: indexmap::IndexMap<String, usize> = indexmap::IndexMap::new();
            if let Some(by_type) = plan.terraform_json["resource"].as_object() {
                for (type_id, by_name) in by_type {
                    let n = by_name.as_object().map_or(0, serde_json::Map::len);
                    counts.insert(type_id.clone(), n);
                }
            }
            counts.sort_keys();
            let total: usize = counts.values().sum();
            let _ = writeln!(out, "architecture\t{name}");
            let _ = writeln!(out, "interface\t{}", interface_for(name).map(|i| i.name).unwrap_or_else(|| "(none)".into()));
            let _ = writeln!(out, "runtime\t{}", plan.runtime_kind);
            let _ = writeln!(out, "total-resources\t{total}");
            for (type_id, n) in counts {
                let _ = writeln!(out, "  {type_id}\t{n}");
            }
            0
        }
    }
}

fn bundled_source_path(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|p| p.join("lava-architectures").join("architectures").join(format!("{name}.tlisp")))
        .unwrap_or_else(|| std::path::PathBuf::from(format!("{name}.tlisp")))
}

fn render_bundled(
    name: &str,
    bindings: &[String],
    list_bindings: &[String],
    err: &mut dyn Write,
) -> Option<magma_lava::LavaPlan> {
    let path = bundled_source_path(name);
    if !path.exists() {
        let _ = writeln!(
            err,
            "lava: bundled architecture `{name}` not found at {}",
            path.display()
        );
        return None;
    }
    let plan_args = LavaPlanArgs {
        path,
        bindings: parse_bindings(bindings, list_bindings),
        gate_with: None,
        runtime_kind: None,
    };
    match synthesize(&plan_args) {
        Ok(p) => Some(p),
        Err(e) => {
            let _ = writeln!(err, "lava: render `{name}` failed: {e}");
            None
        }
    }
}

fn emit_result_slot(arch_xs: &[lava_eval::Sx], out: &mut dyn Write) {
    use lava_eval::Sx;
    // Walk :result keyword to grab its body.
    let mut i = 2;
    while i + 1 < arch_xs.len() {
        if arch_xs[i].as_kw() == Some("result") {
            if let Sx::List(items) = &arch_xs[i + 1] {
                // First atom is the result-name; the rest are :key value pairs.
                if let Some(result_name) = items.first().and_then(Sx::as_sym) {
                    let _ = writeln!(out, "result\t{result_name}");
                }
                let mut j = 1;
                while j + 1 < items.len() {
                    if let Some(k) = items[j].as_kw() {
                        let _ = writeln!(out, "  :{k}");
                    }
                    j += 2;
                }
            }
            return;
        }
        i += 2;
    }
    let _ = writeln!(out, "(no :result clause)");
}

fn emit_plan(
    plan: &magma_lava::LavaPlan,
    format: OutFormat,
    target: Option<&std::path::PathBuf>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let serialized = match format {
        OutFormat::Json => serde_json::to_string_pretty(&plan.terraform_json).unwrap_or_default(),
        OutFormat::Yaml => serde_yaml::to_string(&plan.terraform_json).unwrap_or_default(),
        OutFormat::Crossplane => match plan.crossplane_yaml() {
            Ok(s) => s,
            Err(e) => {
                let _ = writeln!(err, "lava emit (crossplane) failed: {e}");
                return 1;
            }
        },
    };
    match target {
        Some(path) => match std::fs::write(path, &serialized) {
            Ok(()) => {
                let _ = writeln!(out, "wrote {} bytes → {}", serialized.len(), path.display());
                0
            }
            Err(e) => {
                let _ = writeln!(err, "lava write failed: {e}");
                1
            }
        },
        None => {
            let _ = out.write_all(serialized.as_bytes());
            let _ = out.write_all(b"\n");
            0
        }
    }
}

fn parse_bindings(
    scalars: &[String],
    lists: &[String],
) -> IndexMap<String, magma_lava::Binding> {
    let mut out: IndexMap<String, magma_lava::Binding> = IndexMap::new();
    for kv in scalars {
        if let Some((k, v)) = kv.split_once('=') {
            out.insert(k.to_string(), magma_lava::Binding::Scalar(v.to_string()));
        }
    }
    for kv in lists {
        if let Some((k, v)) = kv.split_once('=') {
            let items: Vec<String> = v.split(',').map(std::string::ToString::to_string).collect();
            out.insert(k.to_string(), magma_lava::Binding::List(items));
        }
    }
    out
}

fn count_resources(json: &serde_json::Value) -> usize {
    let Some(by_type) = json.get("resource").and_then(serde_json::Value::as_object) else {
        return 0;
    };
    by_type
        .values()
        .filter_map(serde_json::Value::as_object)
        .map(serde_json::Map::len)
        .sum()
}
