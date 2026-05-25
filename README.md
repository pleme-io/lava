# lava

Operator CLI for the lava IaC suite. Renders, validates, and lists
typed `.tlisp` infrastructure architectures.

## Subcommands

```
lava plan     <PATH>  [--binding K=V] [--binding-list K=V,V,...] [--gate <iface>] [--out FILE] [--format json|yaml]
lava render   <NAME>  [--binding K=V] [--binding-list K=V,V,...]                   [--out FILE] [--format json|yaml]
lava validate <PATH>  [--binding K=V] [--binding-list K=V,V,...]  --gate <iface>
lava ls architectures
lava ls interfaces
lava show interface <NAME>
```

## Examples

```bash
# Render a custom .tlisp to terraform.json on stdout
lava plan ./infra/aws-vpc.tlisp

# Override bindings + schema-gate against the bundled interface
lava plan ./infra/cloudflare.tlisp \
  --binding zone-id=11112222333344445555666677778888 \
  --gate cloudflare-dns-records

# Render a bundled architecture as YAML
lava render aws-vpc-network --format yaml

# Validate inputs against an interface (exit 0/non-zero)
lava validate ./infra/cloudflare.tlisp \
  --binding zone-id=11112222... \
  --gate cloudflare-dns-records

# Inspect the typed Interface JSON for a bundled architecture
lava show interface aws-vpc-network

# Catalog
lava ls architectures
lava ls interfaces
```

## How it works

```text
argv → clap parse
     → magma-lava::synthesize
       → pick_runtime_for_path / LavaRuntime::evaluate_path_with_schema
       → Architecture
       → Synthesizer<TerraformJson>
       → serde_json::Value
     → emit (json | yaml, stdout | --out FILE)
```

No shell, no IPC, no disk-roundtrip from .tlisp to terraform.json.

## Tests

`cargo test --release` — 10 integration tests against the captured-
writer entry point (`lava::cli::run_with_writers`). Covers every
subcommand happy-path + every typed-error path.
