# Institutional Namespace

Give a project a place for its original records, the decisions drawn from them, and the work those decisions produce. This repository pairs a nine-folder workspace template with a Rust checker that reads the workspace’s own manifest and naming catalog.

## Follow one record

Start with the invented [source note](template/06_memory/records/example-note.md), then read its [summary](template/06_memory/summary.md). They live in different places because they serve different readers. The manifest marks the original records as protected, so automatic fixes leave the evidence intact. The [Atlas product page](template/04_products/example/README.md) shows where a reader meets the resulting project.

## Catch a mistake

From the repository root, use Rust 1.94 or later and Cargo. The first build fetches locked dependencies.

```sh
export CARGO_TARGET_DIR="$(mktemp -d)"
cargo build --manifest-path tool/Cargo.toml --locked --bin ovrin-ns
OVRIN_NS_BIN="$CARGO_TARGET_DIR/debug/ovrin-ns" sh examples/check.sh
```

The example copies the template to a temporary directory, checks the valid Atlas page, then changes its heading to **Atlass**. Success means the checker accepts the first page and identifies the misspelling on the second. The final line reads `Valid example passed; invalid spelling was reported.`

## Make the layout yours

Copy the contents of [template](template/) into an empty workspace. Edit [the manifest](template/ovrin.build.json) to assign folders to roles and [the names catalog](template/01_governance/registry/names.toml) to declare your project names. Use structural preflight to check that the declared folders exist. Quiet-process and record-census checks apply during post-migration validation.

Read the [layout guide](docs/LAYOUT.md) for source and runtime boundaries and the [checker manual](tool/README.md) for commands and validation contracts. Run the tool tests with `cargo test --manifest-path tool/Cargo.toml --locked`. Dependency licensing is documented in [third-party notices](THIRD_PARTY.md).

---

**[Explore OVRIN Labs](https://ovrinlabs.ca/)** · [Citium research](https://citium.ovrinlabs.ca/) · [Zavent data tools](https://zaventdata.com/)

[Apache-2.0 license](LICENSE) · [Contributing](CONTRIBUTING.md) · [Report a vulnerability privately](SECURITY.md)
