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
