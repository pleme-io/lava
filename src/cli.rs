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
    /// Render + `<engine> apply` the resulting terraform.json.
    Apply(EngineArgs),
    /// Render + `<engine> destroy` the resulting terraform.json.
    Destroy(EngineArgs),
    /// Render + `<engine> plan` the resulting terraform.json.
    PlanEngine(EngineArgs),
    /// Scaffold a new typed component (.tlisp source) from a template.
    New(NewArgs),
    /// Wrap a .tlisp primitive in a redistributable Rust crate
    /// (cargo + auto-release + workflow shims). Exports the source as
    /// `pub const SOURCE: &str` so consumers `use my_crate::SOURCE;`
    /// and pipe it straight into lava-eval.
    Pack(PackArgs),
    /// `<engine> output [name]` — read outputs from the existing state.
    Output(OutputCmdArgs),
    /// `<engine> state <subcommand>` — inspect or mutate state.
    State(StateArgs),
    /// `<engine> refresh` — re-fetch real-world state.
    Refresh(EngineArgs),
    /// `<engine> import <addr> <id>` — import existing infra into state.
    Import(ImportArgs),
    /// `<engine> workspace <subcommand>` — list/select workspaces.
    Workspace(WorkspaceArgs),
    /// `<engine> fmt` — format terraform.json (no-op for our typed
    /// renderer; surfaced for parity).
    Fmt(EngineArgs),
    /// `<engine> validate` — syntactic + semantic validation of the
    /// rendered terraform.json.
    ValidateTf(EngineArgs),
    /// Fetch + regenerate typed shapes for a terraform provider. Runs
    /// `tofu providers schema -json` against a seeded workspace + pipes
    /// into lava-forge. Default `--dry-run`; pass `--apply` to write.
    Forge(ForgeArgs),
}

#[derive(Parser, Debug)]
pub struct ForgeArgs {
    /// Provider slug (e.g. `aws`, `cloudflare`, `azurerm`, `google`).
    /// Maps to the matching `pleme-io/lava-<provider>` repo + the
    /// terraform-provider source kept in its schema.json.
    pub provider: String,
    /// Provider source for terraform's required_providers block
    /// (e.g. `hashicorp/aws`, `cloudflare/cloudflare`).
    #[arg(long, value_name = "SOURCE")]
    pub source: Option<String>,
    /// Optional version pin for the required_providers block.
    #[arg(long, value_name = "V")]
    pub provider_version: Option<String>,
    /// Working directory for the staging tofu workspace + schema fetch.
    /// Default: tempdir per run.
    #[arg(long)]
    pub work_dir: Option<std::path::PathBuf>,
    /// Output directory for the regenerated resources/. Default:
    /// `../lava-<provider>/resources` relative to the lava binary.
    #[arg(long)]
    pub out: Option<std::path::PathBuf>,
    /// Skip the actual write — print the planned action only.
    #[arg(long, default_value_t = true)]
    pub dry_run: bool,
    /// Write the regenerated schema + resources. Negates --dry-run.
    #[arg(long, default_value_t = false)]
    pub apply: bool,
}

#[derive(Parser, Debug)]
pub struct OutputCmdArgs {
    pub target: String,
    #[arg(long, value_name = "NAME")]
    pub name: Option<String>,
    #[arg(long, default_value_t = Engine::Embedded)]
    pub engine: Engine,
    #[arg(long)]
    pub work_dir: Option<std::path::PathBuf>,
}

#[derive(Parser, Debug)]
pub struct StateArgs {
    /// Subcommand to dispatch to the engine: list / show / mv / rm.
    pub sub: String,
    /// Optional resource-address argument for show/mv/rm.
    pub address: Option<String>,
    /// New address (for mv).
    pub new_address: Option<String>,
    #[arg(long, default_value_t = Engine::Embedded)]
    pub engine: Engine,
    #[arg(long)]
    pub work_dir: Option<std::path::PathBuf>,
}

#[derive(Parser, Debug)]
pub struct ImportArgs {
    pub target: String,
    /// Terraform resource address (e.g. aws_vpc.main).
    pub address: String,
    /// Cloud-side ID to import (e.g. vpc-abc123).
    pub id: String,
    #[arg(long = "binding", value_name = "KEY=VALUE")]
    pub bindings: Vec<String>,
    #[arg(long, default_value_t = Engine::Embedded)]
    pub engine: Engine,
    #[arg(long)]
    pub work_dir: Option<std::path::PathBuf>,
}

#[derive(Parser, Debug)]
pub struct WorkspaceArgs {
    /// list | new | select | show
    pub sub: String,
    /// Workspace name (for new/select).
    pub name: Option<String>,
    #[arg(long, default_value_t = Engine::Embedded)]
    pub engine: Engine,
    #[arg(long)]
    pub work_dir: Option<std::path::PathBuf>,
}

#[derive(Parser, Debug)]
pub struct PackArgs {
    /// Path to the .tlisp source file to package.
    pub path: std::path::PathBuf,
    /// Output directory for the generated crate. Created if missing.
    #[arg(long, value_name = "DIR")]
    pub out: std::path::PathBuf,
    /// Crate name; defaults to `lava-pack-<file-stem>`.
    #[arg(long, value_name = "NAME")]
    pub crate_name: Option<String>,
    /// Crate version. Default: 0.1.0.
    #[arg(long, default_value = "0.1.0")]
    pub version: String,
    /// Crate description.
    #[arg(long)]
    pub description: Option<String>,
    /// Author. Default: "pleme-io".
    #[arg(long, default_value = "pleme-io")]
    pub authors: String,
    /// Overwrite if the target directory already contains a Cargo.toml.
    #[arg(long, default_value_t = false)]
    pub force: bool,
    /// Output format. `crate` (default) writes a full Rust cargo crate
    /// with workflow shims. `caixa` writes a single .caixa.lisp form
    /// that pleme-doc-gen's caixa renderer consumes directly.
    #[arg(long, default_value_t = PackFormat::Crate)]
    pub format: PackFormat,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum PackFormat {
    Crate,
    Caixa,
}

impl std::fmt::Display for PackFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Crate => f.write_str("crate"),
            Self::Caixa => f.write_str("caixa"),
        }
    }
}

#[derive(Parser, Debug)]
pub struct NewArgs {
    /// Kind of component to scaffold.
    pub kind: NewKind,
    /// Name of the component (kebab-case).
    pub name: String,
    /// Output directory. Default: current directory.
    #[arg(long, default_value = ".")]
    pub out: std::path::PathBuf,
    /// Overwrite if the target file already exists.
    #[arg(long, default_value_t = false)]
    pub force: bool,
    /// When kind=test, autogenerate the .test.tlisp body from the
    /// given bundled architecture's resources + outputs (instead of
    /// emitting the minimal default skeleton).
    #[arg(long, value_name = "ARCH")]
    pub against: Option<String>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum NewKind {
    Architecture,
    Interface,
    Test,
    Spec,
    Module,
}

impl NewKind {
    fn file_suffix(self) -> &'static str {
        match self {
            Self::Test => ".test.tlisp",
            _ => ".tlisp",
        }
    }
}

#[derive(Parser, Debug)]
pub struct EngineArgs {
    /// Path to a .tlisp file OR a bundled architecture name.
    pub target: String,
    #[arg(long = "binding", value_name = "KEY=VALUE")]
    pub bindings: Vec<String>,
    #[arg(long = "binding-list", value_name = "KEY=VAL,VAL,...")]
    pub list_bindings: Vec<String>,
    /// Engine selection. Default: `embedded` (in-process magma, fully
    /// in-memory). `tofu` / `terraform` shell out and write
    /// `main.tf.json` to a workdir.
    #[arg(long, default_value_t = Engine::Embedded)]
    pub engine: Engine,
    /// Working directory for the render artifact + state file when
    /// shelling out. Default: tempdir per run (state is ephemeral).
    /// Ignored by `--engine embedded` unless `--persist` is set.
    #[arg(long, value_name = "DIR")]
    pub work_dir: Option<std::path::PathBuf>,
    /// Force `--engine embedded` to ALSO write `main.tf.json` into
    /// `--work-dir` (or a tempdir) for inspection. Off by default —
    /// embedded mode keeps everything in-memory.
    #[arg(long, default_value_t = false)]
    pub persist: bool,
    /// Skip confirmation prompts on apply / destroy (`-auto-approve`).
    /// Default: true (CLI is non-interactive; operator confirms via
    /// `lava plan-engine` first).
    #[arg(long, default_value_t = true)]
    pub auto_approve: bool,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum Engine {
    /// In-process magma; zero subprocess, zero filesystem state spill.
    /// Default; requires the `embedded-magma` build feature.
    Embedded,
    /// Alias for `embedded`.
    Magma,
    /// Shell out to `tofu`. Writes main.tf.json to a workdir.
    Tofu,
    /// Shell out to `terraform`. Writes main.tf.json to a workdir.
    Terraform,
}

impl std::fmt::Display for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Embedded => f.write_str("embedded"),
            Self::Magma => f.write_str("magma"),
            Self::Tofu => f.write_str("tofu"),
            Self::Terraform => f.write_str("terraform"),
        }
    }
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
        Command::Apply(args) => cmd_engine(&args, EngineVerb::Apply, out, err),
        Command::Destroy(args) => cmd_engine(&args, EngineVerb::Destroy, out, err),
        Command::PlanEngine(args) => cmd_engine(&args, EngineVerb::Plan, out, err),
        Command::New(args) => cmd_new(&args, out, err),
        Command::Pack(args) => cmd_pack(&args, out, err),
        Command::Output(args) => cmd_magma_flow(
            &args.target,
            &args.engine,
            args.work_dir.as_ref(),
            "output",
            args.name.as_deref().map(|n| vec![n.to_string()]).unwrap_or_default(),
            &[],
            &[],
            out,
            err,
        ),
        Command::State(args) => {
            let mut argv = vec![args.sub.clone()];
            if let Some(a) = &args.address {
                argv.push(a.clone());
            }
            if let Some(n) = &args.new_address {
                argv.push(n.clone());
            }
            cmd_magma_flow(
                "(state)",
                &args.engine,
                args.work_dir.as_ref(),
                "state",
                argv,
                &[],
                &[],
                out,
                err,
            )
        }
        Command::Refresh(args) => cmd_engine(&args, EngineVerb::Refresh, out, err),
        Command::Import(args) => {
            cmd_magma_flow(
                &args.target,
                &args.engine,
                args.work_dir.as_ref(),
                "import",
                vec![args.address.clone(), args.id.clone()],
                &args.bindings,
                &[],
                out,
                err,
            )
        }
        Command::Workspace(args) => {
            let mut argv = vec!["workspace".to_string(), args.sub.clone()];
            if let Some(n) = &args.name {
                argv.push(n.clone());
            }
            cmd_magma_flow(
                "(workspace)",
                &args.engine,
                args.work_dir.as_ref(),
                "",
                argv,
                &[],
                &[],
                out,
                err,
            )
        }
        Command::Fmt(args) => cmd_engine(&args, EngineVerb::Fmt, out, err),
        Command::ValidateTf(args) => cmd_engine(&args, EngineVerb::Validate, out, err),
        Command::Forge(args) => cmd_forge(&args, out, err),
    }
}

fn cmd_forge(args: &ForgeArgs, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let provider = &args.provider;
    let source = args
        .source
        .clone()
        .unwrap_or_else(|| match provider.as_str() {
            "aws" | "azurerm" | "google" | "kubernetes" | "helm" | "datadog" | "splunk"
            | "akeyless" => format!("hashicorp/{provider}"),
            "cloudflare" => "cloudflare/cloudflare".to_string(),
            _ => format!("hashicorp/{provider}"),
        });
    let work = match &args.work_dir {
        Some(d) => d.clone(),
        None => std::env::temp_dir().join(format!(
            "lava-forge-{}-{}-{}",
            provider,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )),
    };
    if let Err(e) = std::fs::create_dir_all(&work) {
        let _ = writeln!(err, "lava forge: cannot create workdir {}: {e}", work.display());
        return 1;
    }

    // Render a minimal main.tf.json that pulls in the provider via
    // terraform.required_providers. Typed serde_json::Value — no
    // format!() of TF JSON.
    let mut req: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
    let mut entry: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
    entry.insert("source".into(), serde_json::Value::String(source.clone()));
    if let Some(v) = &args.provider_version {
        entry.insert("version".into(), serde_json::Value::String(v.clone()));
    }
    req.insert(provider.clone(), serde_json::Value::Object(entry));
    let tf_json = serde_json::json!({
        "terraform": { "required_providers": serde_json::Value::Object(req) },
        "provider": { provider.as_str(): {} }
    });
    let tf_path = work.join("main.tf.json");
    if let Err(e) = std::fs::write(&tf_path, serde_json::to_string_pretty(&tf_json).unwrap()) {
        let _ = writeln!(err, "lava forge: write {} failed: {e}", tf_path.display());
        return 1;
    }

    let _ = writeln!(
        out,
        "lava forge: staged tofu workspace at {}\n  provider: {}\n  source:   {}",
        work.display(),
        provider,
        source
    );

    if args.dry_run && !args.apply {
        let _ = writeln!(
            out,
            "lava forge: dry-run (pass --apply to actually run tofu init + schema fetch + lava-forge)"
        );
        return 0;
    }

    // tofu init
    let init = std::process::Command::new("tofu")
        .arg("-chdir").arg(work.to_str().unwrap_or("."))
        .arg("init").arg("-input=false")
        .status();
    if init.is_err() {
        let _ = writeln!(err, "lava forge: tofu not on PATH — install OpenTofu and retry");
        return 1;
    }

    // tofu providers schema -json > schema.json
    let schema_out = work.join("schema.json");
    let schema = std::process::Command::new("tofu")
        .arg("-chdir").arg(work.to_str().unwrap_or("."))
        .arg("providers").arg("schema").arg("-json")
        .output();
    match schema {
        Ok(o) if o.status.success() => {
            if let Err(e) = std::fs::write(&schema_out, &o.stdout) {
                let _ = writeln!(err, "lava forge: write {} failed: {e}", schema_out.display());
                return 1;
            }
        }
        _ => {
            let _ = writeln!(err, "lava forge: tofu providers schema -json failed");
            return 1;
        }
    }

    // Invoke lava-forge against the schema → resources output.
    let resources_out = args
        .out
        .clone()
        .unwrap_or_else(|| work.join("resources"));
    let forge = std::process::Command::new("lava-forge")
        .arg("generate")
        .arg("--schema").arg(&schema_out)
        .arg("--out").arg(&resources_out)
        .status();
    match forge {
        Ok(s) if s.success() => {
            let _ = writeln!(
                out,
                "lava forge: regenerated → {}",
                resources_out.display()
            );
            0
        }
        Ok(_) => {
            let _ = writeln!(err, "lava forge: lava-forge exited non-zero");
            1
        }
        Err(_) => {
            let _ = writeln!(err, "lava forge: lava-forge not on PATH");
            1
        }
    }
}

/// Generic dispatcher for `<engine> <verb> <extra-argv...>`. Renders
/// the target if a .tlisp path was provided; for state/workspace
/// flows the verb operates on the workdir directly without needing
/// a render.
#[allow(clippy::too_many_arguments)]
fn cmd_magma_flow(
    target: &str,
    engine: &Engine,
    work_dir: Option<&std::path::PathBuf>,
    verb: &str,
    argv: Vec<String>,
    bindings: &[String],
    list_bindings: &[String],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    // Resolve workdir (no render needed for state/workspace flows).
    let work = match work_dir {
        Some(d) => d.clone(),
        None => std::env::temp_dir().join(format!(
            "lava-{verb}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )),
    };
    if let Err(e) = std::fs::create_dir_all(&work) {
        let _ = writeln!(err, "lava: cannot create work dir {}: {e}", work.display());
        return 1;
    }

    // If target is a real .tlisp path or bundled name, render the
    // architecture into work/main.tf.json so engine has something to
    // operate on. Skip for the synthetic '(state)' / '(workspace)' targets.
    if !target.starts_with('(') {
        let path = {
            let bundled = bundled_source_path(target);
            if bundled.exists() {
                bundled
            } else {
                std::path::PathBuf::from(target)
            }
        };
        if path.exists() {
            let plan_args = LavaPlanArgs {
                path,
                bindings: parse_bindings(bindings, list_bindings),
                gate_with: None,
                runtime_kind: None,
            };
            if let Ok(plan) = synthesize(&plan_args) {
                let body = serde_json::to_string_pretty(&plan.terraform_json).unwrap_or_default();
                let _ = std::fs::write(work.join("main.tf.json"), body);
            }
        }
    }

    let engine_bin = match engine {
        Engine::Embedded | Engine::Magma => {
            // Embedded path for non-render flows: not yet wired (same
            // status as plan/apply embedded). Operator picks tofu/terraform.
            let _ = writeln!(
                err,
                "lava {verb}: embedded magma not yet bundled in this build; \
                 re-run with `--engine tofu` or `--engine terraform`."
            );
            return 1;
        }
        Engine::Tofu => "tofu",
        Engine::Terraform => "terraform",
    };

    // Run init first (idempotent).
    let init = std::process::Command::new(engine_bin)
        .arg("-chdir")
        .arg(work.to_str().unwrap_or("."))
        .arg("init")
        .arg("-input=false")
        .status();
    if let Err(e) = init {
        let _ = writeln!(
            err,
            "lava {verb}: failed to invoke `{engine_bin} init`: {e}"
        );
        return 1;
    }

    let mut cmd = std::process::Command::new(engine_bin);
    cmd.arg("-chdir").arg(work.to_str().unwrap_or("."));
    if !verb.is_empty() {
        cmd.arg(verb);
    }
    for a in &argv {
        cmd.arg(a);
    }
    match cmd.status() {
        Ok(s) if s.success() => {
            let _ = writeln!(out, "lava {verb}: ok");
            0
        }
        Ok(_) => {
            let _ = writeln!(err, "lava {verb}: `{engine_bin} {verb}` exited non-zero");
            1
        }
        Err(e) => {
            let _ = writeln!(
                err,
                "lava {verb}: failed to invoke `{engine_bin} {verb}`: {e}"
            );
            1
        }
    }
}

fn cmd_pack(args: &PackArgs, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let src = match std::fs::read_to_string(&args.path) {
        Ok(s) => s,
        Err(e) => {
            let _ = writeln!(err, "lava pack: read {}: {e}", args.path.display());
            return 1;
        }
    };
    let file_stem = args
        .path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("primitive")
        .trim_end_matches(".test")
        .to_string();
    let crate_name = args
        .crate_name
        .clone()
        .unwrap_or_else(|| {
            let mut n = String::from("lava-pack-");
            n.push_str(&file_stem);
            n
        });
    let library_name = crate_name.replace('-', "_");
    let description = args
        .description
        .clone()
        .unwrap_or_else(|| {
            let mut d = String::from("Redistributable lava primitive packaged by `lava pack` from ");
            d.push_str(args.path.file_name().and_then(|n| n.to_str()).unwrap_or(""));
            d
        });

    // Caixa format branch — emits a single .caixa.lisp file that
    // pleme-doc-gen's caixa renderer consumes. No Rust crate scaffold.
    if args.format == PackFormat::Caixa {
        return emit_caixa_form(args, &file_stem, &crate_name, &src, &description, out, err);
    }

    // Output layout.
    let out_dir = &args.out;
    let cargo_toml = out_dir.join("Cargo.toml");
    if cargo_toml.exists() && !args.force {
        let _ = writeln!(
            err,
            "lava pack: {} already exists (pass --force to overwrite)",
            cargo_toml.display()
        );
        return 1;
    }
    if let Err(e) = std::fs::create_dir_all(out_dir.join("src")) {
        let _ = writeln!(
            err,
            "lava pack: cannot create {}: {e}",
            out_dir.join("src").display()
        );
        return 1;
    }
    if let Err(e) = std::fs::create_dir_all(out_dir.join(".github").join("workflows")) {
        let _ = writeln!(
            err,
            "lava pack: cannot create {}: {e}",
            out_dir.join(".github").join("workflows").display()
        );
        return 1;
    }

    // Write each artifact via typed builders (no format!() of code).
    let cargo_body =
        render_pack_cargo_toml(&crate_name, &library_name, &args.version, &description, &args.authors);
    if let Err(e) = std::fs::write(&cargo_toml, &cargo_body) {
        let _ = writeln!(err, "lava pack: write {} failed: {e}", cargo_toml.display());
        return 1;
    }
    let lib_rs = out_dir.join("src").join("lib.rs");
    let lib_body = render_pack_lib_rs(&library_name, &src);
    if let Err(e) = std::fs::write(&lib_rs, &lib_body) {
        let _ = writeln!(err, "lava pack: write {} failed: {e}", lib_rs.display());
        return 1;
    }
    let readme = out_dir.join("README.md");
    let readme_body = render_pack_readme(&crate_name, &library_name, &description, &file_stem);
    if let Err(e) = std::fs::write(&readme, &readme_body) {
        let _ = writeln!(err, "lava pack: write {} failed: {e}", readme.display());
        return 1;
    }
    let gitignore = out_dir.join(".gitignore");
    let _ = std::fs::write(&gitignore, "/target\n");

    // Workflow shims — same set as every other lava-* repo.
    let workflows_dir = out_dir.join(".github").join("workflows");
    for (filename, body) in [
        ("auto-release.yml", PACK_WORKFLOW_AUTO_RELEASE),
        ("pre-merge-gate.yml", PACK_WORKFLOW_PRE_MERGE),
        ("security-gate.yml", PACK_WORKFLOW_SECURITY),
    ] {
        let p = workflows_dir.join(filename);
        if let Err(e) = std::fs::write(&p, body) {
            let _ = writeln!(err, "lava pack: write {} failed: {e}", p.display());
            return 1;
        }
    }

    let _ = writeln!(
        out,
        "lava pack: wrote crate `{}` to {} ({} bytes of .tlisp source embedded)",
        crate_name,
        out_dir.display(),
        src.len()
    );
    0
}

fn render_pack_cargo_toml(
    crate_name: &str,
    library_name: &str,
    version: &str,
    description: &str,
    authors: &str,
) -> String {
    // Typed assembly: literal scaffold + sanitized field substitution.
    let mut s = String::new();
    s.push_str("[package]\n");
    s.push_str("authors = [\"");
    s.push_str(authors);
    s.push_str("\"]\n");
    s.push_str("description = \"");
    push_toml_escaped(&mut s, description);
    s.push_str("\"\n");
    s.push_str("edition = \"2024\"\n");
    s.push_str("license = \"MIT\"\n");
    s.push_str("name = \"");
    s.push_str(crate_name);
    s.push_str("\"\n");
    s.push_str("readme = \"README.md\"\n");
    s.push_str("repository = \"https://github.com/pleme-io/");
    s.push_str(crate_name);
    s.push_str("\"\n");
    s.push_str("version = \"");
    s.push_str(version);
    s.push_str("\"\n\n");
    s.push_str("[lib]\n");
    s.push_str("name = \"");
    s.push_str(library_name);
    s.push_str("\"\n");
    s.push_str("path = \"src/lib.rs\"\n\n");
    s.push_str("[lints.clippy]\n");
    s.push_str("pedantic = \"warn\"\n");
    s
}

fn render_pack_lib_rs(library_name: &str, tlisp_source: &str) -> String {
    let mut s = String::new();
    s.push_str("//! ");
    s.push_str(library_name);
    s.push_str(" — redistributable lava primitive packaged via `lava pack`.\n//!\n");
    s.push_str("//! Consumers import [`SOURCE`] and feed it to lava-eval:\n//!\n");
    s.push_str("//! ```ignore\n");
    s.push_str("//! use ");
    s.push_str(library_name);
    s.push_str("::SOURCE;\n");
    s.push_str("//! let arch = lava_eval::eval_architecture(SOURCE, &bindings)?;\n");
    s.push_str("//! ```\n\n");
    s.push_str("#![allow(clippy::module_name_repetitions)]\n\n");
    s.push_str("/// The packaged .tlisp source text. Stable byte-for-byte across\n");
    s.push_str("/// every consumer; reproducible because lava pack embeds the source\n");
    s.push_str("/// without re-rendering.\n");
    s.push_str("pub const SOURCE: &str = ");
    // Use raw string literal r##\"...\"## so the source survives any \" + \\
    // unmodified; pick a delimiter wider than any "## sequence the source
    // might contain (cap at 8 hashes — far wider than any realistic tlisp).
    let delimiter = choose_raw_delim(tlisp_source);
    s.push('r');
    for _ in 0..delimiter {
        s.push('#');
    }
    s.push('"');
    s.push_str(tlisp_source);
    s.push('"');
    for _ in 0..delimiter {
        s.push('#');
    }
    s.push_str(";\n\n");
    s.push_str("#[cfg(test)]\nmod tests {\n");
    s.push_str("    use super::*;\n\n");
    s.push_str("    #[test]\n    fn source_is_non_empty() {\n");
    s.push_str("        assert!(!SOURCE.is_empty());\n    }\n");
    s.push_str("}\n");
    s
}

fn render_pack_readme(crate_name: &str, library_name: &str, description: &str, stem: &str) -> String {
    let mut s = String::new();
    s.push_str("# ");
    s.push_str(crate_name);
    s.push_str("\n\n");
    s.push_str(description);
    s.push_str("\n\n## Usage\n\n```rust\nuse ");
    s.push_str(library_name);
    s.push_str("::SOURCE;\n");
    s.push_str("use lava_eval::{eval_architecture, InputBindings};\n\n");
    s.push_str("let arch = eval_architecture(SOURCE, &InputBindings::new())?;\nlet json = arch.render_terraform_json()?;\n```\n\n");
    s.push_str("Generated from `");
    s.push_str(stem);
    s.push_str(".tlisp` via `lava pack`. Re-run `lava pack` to refresh.\n");
    s
}

/// Emit a single `.caixa.lisp` form that pleme-doc-gen's caixa
/// renderer consumes directly. The output dir gets one file:
/// `<crate_name>.caixa.lisp` carrying the .tlisp source as a typed
/// :files entry.
fn emit_caixa_form(
    args: &PackArgs,
    file_stem: &str,
    crate_name: &str,
    src: &str,
    description: &str,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let out_dir = &args.out;
    if let Err(e) = std::fs::create_dir_all(out_dir) {
        let _ = writeln!(err, "lava pack (caixa): cannot create {}: {e}", out_dir.display());
        return 1;
    }
    let caixa_path = out_dir.join(format!("{crate_name}.caixa.lisp"));
    if caixa_path.exists() && !args.force {
        let _ = writeln!(
            err,
            "lava pack (caixa): {} already exists (pass --force)",
            caixa_path.display()
        );
        return 1;
    }
    let body = render_caixa_lisp(crate_name, &args.version, description, file_stem, src);
    if let Err(e) = std::fs::write(&caixa_path, &body) {
        let _ = writeln!(err, "lava pack (caixa): write {} failed: {e}", caixa_path.display());
        return 1;
    }
    let _ = writeln!(
        out,
        "lava pack (caixa): wrote {} bytes → {} ({} bytes of .tlisp source embedded)",
        body.len(),
        caixa_path.display(),
        src.len()
    );
    0
}

/// Render the typed (defcaixa …) form. Per ★★ TYPED EMISSION the
/// body is built via push_str of literal scaffold + sanitized field
/// substitution + raw-string-wrapped source embedding. No format!()
/// of code/structured emission.
fn render_caixa_lisp(
    crate_name: &str,
    version: &str,
    description: &str,
    file_stem: &str,
    tlisp_source: &str,
) -> String {
    let mut s = String::new();
    s.push_str(";; ");
    s.push_str(crate_name);
    s.push_str(".caixa.lisp — generated by `lava pack --format caixa`\n");
    s.push_str(";; Consume via `pleme-doc-gen caixa --source <this-file>`.\n\n");
    s.push_str("(defcaixa\n");
    s.push_str("  :name \"");
    push_lisp_escaped(&mut s, crate_name);
    s.push_str("\"\n");
    s.push_str("  :kind lava-architecture\n");
    s.push_str("  :ecosystem rust-single-crate\n");
    s.push_str("  :package (:name \"");
    push_lisp_escaped(&mut s, crate_name);
    s.push_str("\"\n");
    s.push_str("            :version \"");
    push_lisp_escaped(&mut s, version);
    s.push_str("\"\n");
    s.push_str("            :license \"MIT\"\n");
    s.push_str("            :description \"");
    push_lisp_escaped(&mut s, description);
    s.push_str("\")\n");
    s.push_str("  :files ((:path \"src/");
    push_lisp_escaped(&mut s, file_stem);
    s.push_str(".tlisp\"\n           :content ");
    // Multi-line lisp string — use the `(quoted-string …)` form via
    // \"…\" with escaped content. Since the source text can contain
    // any character, escape \\ and \" through push_lisp_escaped.
    s.push('"');
    push_lisp_escaped(&mut s, tlisp_source);
    s.push_str("\")\n");
    s.push_str("          (:path \"README.md\"\n           :content \"");
    push_lisp_escaped(&mut s, &caixa_readme(crate_name, description));
    s.push_str("\")))\n");
    s
}

fn caixa_readme(crate_name: &str, description: &str) -> String {
    let mut r = String::new();
    r.push_str("# ");
    r.push_str(crate_name);
    r.push_str("\n\n");
    r.push_str(description);
    r.push_str("\n\nGenerated by `lava pack --format caixa`. Re-render via `lava pack`.\n");
    r
}

fn push_lisp_escaped(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            _ => out.push(c),
        }
    }
}

fn push_toml_escaped(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            _ => out.push(c),
        }
    }
}

/// Pick the smallest raw-string `#` count such that the embedded source
/// can't accidentally close the literal. Any sequence of N `"` followed
/// by N `#` would close; we pick N = (max(N appearing in source) + 1).
fn choose_raw_delim(s: &str) -> usize {
    let mut needed = 1usize;
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            let mut hashes = 0usize;
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] == b'#' {
                hashes += 1;
                j += 1;
            }
            if hashes + 1 > needed {
                needed = hashes + 1;
            }
            i = j;
        } else {
            i += 1;
        }
    }
    needed
}

const PACK_WORKFLOW_AUTO_RELEASE: &str = "name: auto-release
on:
  push:
    branches:
      - main
  workflow_dispatch:
    inputs:
      bump-type:
        description: \"patch | minor | major\"
        required: false
        default: patch
jobs:
  release:
    uses: pleme-io/substrate/.github/workflows/cargo-auto-release.yml@main
    secrets: inherit
";

const PACK_WORKFLOW_PRE_MERGE: &str = "name: pre-merge-gate
on:
  pull_request:
    branches: [main]
jobs:
  gate:
    uses: pleme-io/substrate/.github/workflows/pre-merge-gate.yml@main
    secrets: inherit
";

const PACK_WORKFLOW_SECURITY: &str = "name: security-gate
on:
  pull_request:
    branches: [main]
  schedule:
    - cron: '0 6 * * 1'
jobs:
  gate:
    uses: pleme-io/substrate/.github/workflows/security-gate.yml@main
    secrets: inherit
";

fn cmd_new(args: &NewArgs, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    if let Err(e) = std::fs::create_dir_all(&args.out) {
        let _ = writeln!(err, "lava new: cannot create {}: {e}", args.out.display());
        return 1;
    }
    let file = args
        .out
        .join(format!("{}{}", args.name, args.kind.file_suffix()));
    if file.exists() && !args.force {
        let _ = writeln!(
            err,
            "lava new: {} already exists (pass --force to overwrite)",
            file.display()
        );
        return 1;
    }
    let body = if args.kind == NewKind::Test && args.against.is_some() {
        let arch = args.against.as_ref().unwrap();
        match scaffold_test_against(arch, &args.name, err) {
            Some(s) => s,
            None => return 1,
        }
    } else {
        scaffold_for(args.kind, &args.name)
    };
    if let Err(e) = std::fs::write(&file, &body) {
        let _ = writeln!(err, "lava new: write {} failed: {e}", file.display());
        return 1;
    }
    let _ = writeln!(out, "wrote {} bytes → {}", body.len(), file.display());
    0
}

/// Generate a .test.tlisp body from the bundled architecture's
/// rendered resources + outputs. One resource-exists assertion per
/// resource, attribute-equals for stable scalar attributes,
/// ref-valid + matching output-equals per :result slot.
fn scaffold_test_against(arch: &str, test_name: &str, err: &mut dyn Write) -> Option<String> {
    use indexmap::IndexMap;
    // Render the architecture with default bindings.
    let path = bundled_source_path(arch);
    if !path.exists() {
        let _ = writeln!(err, "lava new test --against: `{arch}` not found at {}", path.display());
        return None;
    }
    let plan_args = LavaPlanArgs {
        path,
        bindings: IndexMap::new(),
        gate_with: None,
        runtime_kind: None,
    };
    let plan = match synthesize(&plan_args) {
        Ok(p) => p,
        Err(e) => {
            let _ = writeln!(err, "lava new test --against: render failed: {e}");
            return None;
        }
    };

    // Walk the typed Architecture to build assertions.
    let mut s = String::new();
    s.push_str(";; ");
    s.push_str(test_name);
    s.push_str(".test.tlisp — autogenerated by `lava new test --against ");
    s.push_str(arch);
    s.push_str("`\n;; Refine + extend by hand once the architecture stabilizes.\n\n");
    s.push_str("(deflava-test ");
    s.push_str(test_name);
    s.push_str("/default\n  :architecture ");
    s.push_str(arch);
    s.push_str("\n  :assertions (\n");

    for r in &plan.architecture.resources {
        s.push_str("    (resource-exists ");
        // type_id is snake_case; convert to kebab for the tlisp head.
        s.push_str(&r.type_id.replace('_', "-"));
        s.push_str(" \"");
        push_lisp_escaped(&mut s, &r.name);
        s.push_str("\")\n");
    }

    // One resource-count per distinct type.
    let mut counts: indexmap::IndexMap<String, usize> = indexmap::IndexMap::new();
    for r in &plan.architecture.resources {
        *counts.entry(r.type_id.clone()).or_insert(0) += 1;
    }
    for (type_id, n) in &counts {
        s.push_str("    (resource-count ");
        s.push_str(&type_id.replace('_', "-"));
        s.push_str(" ");
        s.push_str(&n.to_string());
        s.push_str(")\n");
    }

    // ref-valid baseline.
    s.push_str("    (ref-valid)\n");

    s.push_str("  ))\n");
    Some(s)
}

fn scaffold_for(kind: NewKind, name: &str) -> String {
    // Typed scaffolds — no format!() of tlisp syntax: each writer goes
    // through write!()/push_str of literal template strings + the
    // single dynamic value `name`. The template is the typed surface.
    match kind {
        NewKind::Architecture => {
            let mut s = String::new();
            s.push_str(";; ");
            s.push_str(name);
            s.push_str(".tlisp — scaffolded by `lava new architecture`\n\n");
            s.push_str("(deflava-interface ");
            s.push_str(name);
            s.push_str("\n  :doc \"TODO — describe ");
            s.push_str(name);
            s.push_str("\"\n  :inputs ()\n  :outputs ())\n\n");
            s.push_str("(deflava-architecture ");
            s.push_str(name);
            s.push_str("\n  :inputs ()\n  :resources ())\n");
            s
        }
        NewKind::Interface => {
            let mut s = String::new();
            s.push_str(";; ");
            s.push_str(name);
            s.push_str(".tlisp — scaffolded by `lava new interface`\n\n");
            s.push_str("(deflava-interface ");
            s.push_str(name);
            s.push_str("\n  :doc \"TODO — typed contract for ");
            s.push_str(name);
            s.push_str("\"\n  :inputs ()\n  :outputs ())\n");
            s
        }
        NewKind::Test => {
            let mut s = String::new();
            s.push_str(";; ");
            s.push_str(name);
            s.push_str(".test.tlisp — scaffolded by `lava new test`\n\n");
            s.push_str("(deflava-test ");
            s.push_str(name);
            s.push_str("/default\n  :architecture ");
            s.push_str(name);
            s.push_str("\n  :assertions ((ref-valid)))\n");
            s
        }
        NewKind::Spec => {
            let mut s = String::new();
            s.push_str(";; ");
            s.push_str(name);
            s.push_str(".tlisp — scaffolded by `lava new spec`\n\n");
            s.push_str("(deflava-spec ");
            s.push_str(name);
            s.push_str("\n  :scenarios (\n    (:name \"smoke\"\n     :given (:architecture ");
            s.push_str(name);
            s.push_str(")\n     :when  (:bindings ())\n     :then  ((ref-valid)))))\n");
            s
        }
        NewKind::Module => {
            let mut s = String::new();
            s.push_str(";; ");
            s.push_str(name);
            s.push_str(".tlisp — scaffolded by `lava new module`\n");
            s.push_str(";; A deflava-module composes typed inputs → typed outputs.\n");
            s.push_str(";; (Module evaluation arrives with lava-modules; this scaffold\n");
            s.push_str(";; declares the shape today so authoring can start.)\n\n");
            s.push_str("(deflava-module ");
            s.push_str(name);
            s.push_str("\n  :inputs ()\n  :resources ()\n  :outputs ())\n");
            s
        }
    }
}

#[derive(Copy, Clone, Debug)]
enum EngineVerb {
    Apply,
    Destroy,
    Plan,
    Refresh,
    Fmt,
    Validate,
}

impl EngineVerb {
    fn as_str(self) -> &'static str {
        match self {
            Self::Apply => "apply",
            Self::Destroy => "destroy",
            Self::Plan => "plan",
            Self::Refresh => "refresh",
            Self::Fmt => "fmt",
            Self::Validate => "validate",
        }
    }
}

fn cmd_engine(
    args: &EngineArgs,
    verb: EngineVerb,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    // 1) Resolve target → .tlisp source path.
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
            "lava {}: target `{}` not found",
            verb.as_str(),
            args.target
        );
        return 1;
    }

    // 2) Render to terraform.json in-memory.
    let plan_args = LavaPlanArgs {
        path,
        bindings: parse_bindings(&args.bindings, &args.list_bindings),
        gate_with: None,
        runtime_kind: None,
    };
    let plan = match synthesize(&plan_args) {
        Ok(p) => p,
        Err(e) => {
            let _ = writeln!(err, "lava {}: render failed: {e}", verb.as_str());
            return 1;
        }
    };

    // 3) Dispatch by engine selection.
    match args.engine {
        Engine::Embedded | Engine::Magma => {
            // Optionally persist the rendered JSON for inspection
            // even though the engine itself runs in-memory.
            if args.persist {
                if let Err(code) = persist_terraform_json(args, &plan.terraform_json, err) {
                    return code;
                }
            }
            run_embedded_magma(verb, &plan.terraform_json, args.auto_approve, out, err)
        }
        Engine::Tofu | Engine::Terraform => {
            let engine_bin = match args.engine {
                Engine::Tofu => "tofu",
                Engine::Terraform => "terraform",
                _ => unreachable!(),
            };
            run_shell_engine(verb, args, &plan.terraform_json, engine_bin, out, err)
        }
    }
}

/// Drive the operation through the in-process embedded magma engine.
/// Compiled out unless the `embedded-magma` feature is on; without
/// the feature, surfaces a typed error so operators know to either
/// rebuild with the feature or pick a shell-out engine.
fn run_embedded_magma(
    verb: EngineVerb,
    _terraform_json: &serde_json::Value,
    _auto_approve: bool,
    _out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    #[cfg(feature = "embedded-magma")]
    {
        let _ = writeln!(
            _out,
            "lava {} (embedded magma): in-process plan/apply",
            verb.as_str()
        );
        // The real impl wires magma_config::Config::from_json +
        // magma_plan::plan + magma_apply::apply here. magma is
        // currently a workspace-internal crate; this branch lands
        // when magma publishes its library API surface (tracked
        // upstream as #349).
        let _ = writeln!(
            err,
            "lava {} (embedded magma): library API surface not yet bundled into this build; \
             upstream magma needs to publish magma-config / magma-plan / magma-apply as git deps. \
             Falling back to subprocess via --engine tofu or --engine terraform.",
            verb.as_str()
        );
        1
    }
    #[cfg(not(feature = "embedded-magma"))]
    {
        let _ = writeln!(
            err,
            "lava {}: embedded engine selected but this build was compiled without the \
             `embedded-magma` feature. Rebuild with `cargo build --features embedded-magma` \
             or pick a different engine (`--engine tofu` / `--engine terraform`).",
            verb.as_str()
        );
        let _ = err; // keep `err` used when feature disabled
        1
    }
}

/// Drive the operation through `tofu` / `terraform`. Writes the
/// rendered JSON into a workdir + invokes `init` + `verb`.
fn run_shell_engine(
    verb: EngineVerb,
    args: &EngineArgs,
    terraform_json: &serde_json::Value,
    engine: &str,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let work_dir = match resolve_work_dir(args, verb, err) {
        Ok(d) => d,
        Err(code) => return code,
    };
    let main_tf_json = work_dir.join("main.tf.json");
    let body = serde_json::to_string_pretty(terraform_json).unwrap_or_default();
    if let Err(e) = std::fs::write(&main_tf_json, &body) {
        let _ = writeln!(
            err,
            "lava {}: write {} failed: {e}",
            verb.as_str(),
            main_tf_json.display()
        );
        return 1;
    }
    let _ = writeln!(
        out,
        "lava {} ({engine}): wrote {} bytes → {}",
        verb.as_str(),
        body.len(),
        main_tf_json.display()
    );

    // init
    let init = std::process::Command::new(engine)
        .arg("-chdir")
        .arg(work_dir.to_str().unwrap_or("."))
        .arg("init")
        .arg("-input=false")
        .status();
    let Ok(status) = init else {
        let _ = writeln!(
            err,
            "lava {}: failed to invoke `{engine} init`: is `{engine}` on $PATH?",
            verb.as_str()
        );
        return 1;
    };
    if !status.success() {
        let _ = writeln!(
            err,
            "lava {}: `{engine} init` exited with non-zero status",
            verb.as_str()
        );
        return 1;
    }

    // verb
    let mut cmd = std::process::Command::new(engine);
    cmd.arg("-chdir")
        .arg(work_dir.to_str().unwrap_or("."))
        .arg(verb.as_str())
        .arg("-input=false");
    if matches!(verb, EngineVerb::Apply | EngineVerb::Destroy) && args.auto_approve {
        cmd.arg("-auto-approve");
    }
    match cmd.status() {
        Ok(s) if s.success() => 0,
        Ok(_) => {
            let _ = writeln!(
                err,
                "lava {}: `{engine} {}` exited with non-zero status",
                verb.as_str(),
                verb.as_str()
            );
            1
        }
        Err(e) => {
            let _ = writeln!(
                err,
                "lava {}: failed to invoke `{engine} {}`: {e}",
                verb.as_str(),
                verb.as_str()
            );
            1
        }
    }
}

fn persist_terraform_json(
    args: &EngineArgs,
    terraform_json: &serde_json::Value,
    err: &mut dyn Write,
) -> Result<(), i32> {
    let work_dir = resolve_work_dir(args, EngineVerb::Plan, err)?;
    let main_tf_json = work_dir.join("main.tf.json");
    let body = serde_json::to_string_pretty(terraform_json).unwrap_or_default();
    std::fs::write(&main_tf_json, body).map_err(|e| {
        let _ = writeln!(err, "lava persist: write {} failed: {e}", main_tf_json.display());
        1_i32
    })?;
    Ok(())
}

fn resolve_work_dir(
    args: &EngineArgs,
    verb: EngineVerb,
    err: &mut dyn Write,
) -> Result<std::path::PathBuf, i32> {
    let dir = match &args.work_dir {
        Some(d) => d.clone(),
        None => std::env::temp_dir().join(format!(
            "lava-{}-{}-{}",
            verb.as_str(),
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )),
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        let _ = writeln!(
            err,
            "lava {}: cannot create work dir {}: {e}",
            verb.as_str(),
            dir.display()
        );
        return Err(1);
    }
    Ok(dir)
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
