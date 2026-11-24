# Namespace checks

The namespace’s living text uses the names and wording established in governance. Records retain their original bytes. The manifest is the map for root roles and record membership.

`ovrin-ns` provides a Rust library and a thin command line. It runs offline without a scripting interpreter. Its governance catalogues are `registry/names.toml` and `registry/slop.toml` under the manifest’s governance role. Record membership follows `records.entries` role-relative globs. A path containing a symbolic link is never followed during document traversal.

Today’s commands:

```text
ovrin-ns --root /path/to/namespace check /path/to/documents --class core --json
ovrin-ns --root /path/to/namespace check /path/to/spines --class spine
ovrin-ns --root /path/to/namespace fix --names /path/to/documents --dry-run --ledger /path/to/changes.tsv > plan.json
ovrin-ns --root /path/to/namespace fix --names /path/to/documents --write --accept plan.json --ledger /path/to/changes.tsv --before /outside/namespace/before
ovrin-ns --root /path/to/namespace fix --paths /path/to/consumers --dry-run --ledger /path/to/paths.tsv
ovrin-ns preflight --mode staged --manifest /path/to/namespace/ovrin.build.json
ovrin-ns preflight --mode pre --manifest /path/to/home/ovrin.build.json
```

Without `--root`, commands use the current directory’s nearest ancestor manifest. An explicit root selects that directory’s manifest. Preflight selects the parent of its manifest as its root; staged mode therefore operates on the staged namespace.

Checks read only `.md` and `.txt` prose. YAML front matter, URLs, link destinations and path tokens remain outside prose matching. Inline, fenced and indented code is inspected separately: correct lowercase technical forms are valid, but listed wrong variants, retired names and applicable slop patterns remain findings. Automatic fixes preserve code bytes. Link labels remain prose. Names match whole tokens, case-sensitively for wrong variants and case-insensitively for retired identities. Longer substitutions take precedence. Joined domain and identifier components retain their spelling. Within `00_constitution/canon_chronicles/`, historical spellings and retired identities are exempt in prose and code, while slop is checked. Outside it, retired identities in prose are findings in core; in code they are findings in both classes; spines check current-name variants. A retired name with an explicit catalogue successor is mechanically fixable; a retired name without one requires review. Source-linked ledger examples are refused as records; source-located block quotations and technical quoted search strings retain their original bytes. List-nested code fences share the same exemption as other fenced code. Explicit records directories, extracted source ledgers, literal replacement maps and command-syntax headings are preserved as evidence or technical data. A names finding identifies its catalogue form as `name:<form>` or `retired:<identity>`, since these catalogue tables have no separate id field. A slop finding uses the entry’s id. Judgment entries remain the auditor’s responsibility. Name-change phrases require a catalogue name in the same sentence; sentences end at a period, question mark or exclamation point, with Markdown blocks keeping separate headings, paragraphs and rows apart. Ordinary lowercase nouns do not establish a project identity for narration context. Mechanical findings still require judgment when an ordinary word could have a technical meaning.

A dry run emits a JSON object with `changes` and `skipped`. To accept a subset, retain only the desired complete objects in `changes`; keep the object’s other fields. A write requires `--accept` and recomputes every selected change. Changed hashes, edited replacements, duplicate entries and protected records are refused before writing. Names mode reads prose documents only and proposes spelling corrections including retired identities with explicit successors. Paths mode reads UTF-8 text, including scripts and settings, while refusing records and listing binary and data-store skips. JSON files are conservatively treated as data stores. Retired roots without a replacement remain findings for review.

Each changed file keeps its original permissions; its hash-named `.before` copy lives under the explicit `--before` folder outside living text, at the same relative path. A home-rooted namespace may use a manifest-excluded record hold, with its journal beside the hold; ordinary internal folders remain refused. Writes replace each file atomically and append a TSV ledger row: epoch seconds, path, before hash, after hash, edit count, backup path. A multi-file apply is sequential; a later I/O error leaves earlier successful files ledgered. The ledger must be outside protected records. Keep accepted plans and ledgers outside the target input set.

Preflight prints P1 through P7, with the number checked and findings under each result. P1 covers roots, division membership, declared structural paths, concrete landing places and record containers. P2 scans declared consumers for retired names, with exact byte offsets for binary path strings. P3 checks declared symbolic links, unit and desktop paths, and relative links in root READMEs and the namespace law. P4 checks root spelling and number uniqueness. P5 counts user-folder literals. P6 lists undeclared host entries. P7 inspects open root paths in post mode; it is informational before migration. Pre mode considers only absent retired roots; staged and post modes consider all retired roots. Missing declared consumer lists are reported as failures rather than silently ignored. The v2 manifest has no consumers list; its fallback covers desktop configuration, user units, command links, installed release binaries, harness guidance and profiles. It does not traverse the entire home folder.

Two commands answer for the namespace as a whole. `ovrin-ns source` is run inside a project folder (or given a path, or `--all`): for each installed tool whose project folder that is, it reads the release's own record through the release index the manifest names (`release_index.path`) and says where the code behind the installed release is: checked out in the folder, in the folder's history (with the branches that hold it and a command to see it), not in the folder but found at the place the index records, not found, or not recorded at all. Exit 1 when any tool's code is not in its folder. `ovrin-ns drift` compares the registry with the disk (preflight P1 to P4 and the top-level entries the manifest does not know, host-listed entries excepted), the namespace law's root map, division memberships and Citium topology with the registry, the layout specification's tree of authored roots with the registry, the house rules' institutional layout with the registry and the disk, and every installed release with its project folder. The four sources are named in the manifest's `drift_check` block. Each disagreement is printed as one line; a line named in the `drift_check.listed` file carries its owner and why; the last line is `no drift` or the count, listed and not listed. Exit 0 only when there is no drift at all.

```text
cd /path/to/project && ovrin-ns source
ovrin-ns source --all --json
ovrin-ns drift
ovrin-ns drift --json
```

Exit codes are 0 for a successful command (including refused records alone), 1 for check findings or a failed gating preflight check, and 2 for invalid inputs or a tool error. JSON preflight emits one object per check; JSON prose checks emit one object with findings, checked-document count and refused records.

Build on Linux with Rust 1.94 or later. From this directory, run `cargo build --locked` and `cargo test --locked`. Cargo fetches the versions in Cargo.lock on the first run. For an offline build, fetch the locked dependencies in advance with `cargo fetch --locked`.

Migration preserves every original and refuses an occupied rename destination. The current commands are:

```text
ovrin-ns move --map MOVE_IN.tsv --journal MOVE_JOURNAL.tsv [--dry-run]
ovrin-ns move --inverse MOVE_JOURNAL.tsv
ovrin-ns switch --set SWITCH.tsv --before /absolute/unused/hold --journal SWITCH_JOURNAL.tsv [--dry-run]
ovrin-ns switch --inverse SWITCH_JOURNAL.tsv
ovrin-ns preflight --mode post --manifest /absolute/namespace/ovrin.build.json
```

Maps have the section 9 headers `seq, from, to, kind, reason`; switch sets use `seq, kind, live, staged, inverse, reason` (tab separated). Sequence numbers are unique unsigned integers and execution is sorted numerically. Paths are absolute, without `..`; `key` means a filesystem key/path rename, never an implicit edit inside a settings format. Staged file contents already carry any settings-key changes. `file`, `unit`, `env`, `trust` and `pack` publish a reflink copy; the original bytes and original file object stay in the before hold. `link` and `release` publish a symbolic link to the absolute staged target. `worktree` invokes native `git -C live worktree repair -- staged`; its `inverse` cell is the absolute path to repair on rollback, after the worktree's move has been reversed. The git administrative folder must remain addressable at `live`. These commands neither reload nor start services. Each operation prints its write effects before execution. Dry runs create no files.

Journals have `time, result, act` columns (tab separated); `act` is a JSON object so filenames cannot break the journal syntax. File-copy and rename acts are recorded and synced before execution; their fixed-width result changes from `pending` to `applied` after success. Each rename uses the kernel's `renameat2(RENAME_NOREPLACE)`, including during inverse runs. No copying fallback is used for cross-filesystem moves. A refusal identifies the sequence and leaves earlier journaled acts available for rollback. Worktree repairs are journaled before invoking git and carry their explicit inverse. Prepared copies, byte backups and original objects remain in the before hold; rollback moves originals back and keeps all evidence. Inverse journals sit beside the forward journal as `.inverse.tsv`; rename inverses may resume after interruption, using completed inverse entries and object fingerprints. Modified files, changed symlink targets and occupied original names are refused. Directory fingerprints identify the original filesystem object, rather than recursively hashing its contents; record content totals are checked separately in post mode.

Post mode also reports P8, record census. `records.census` in the manifest is either a root-relative TSV path or a `{ "role": "memory", "path": "..." }` locator. The TSV header is `path, files, bytes` (tab separated); each row names a concrete declared record container and its pre-move totals. Regular files count their length; symlinks count as one entry and the byte length of their stored target, without following them. Every declared non-host record container must have a covering census row. Missing, empty, incomplete or differing counts fail. Post mode scans environment, trust and key consumer files too, and checks retired names inside symbolic-link targets. It never reads record text for name substitutions.


The checker keeps governed text correct after edits. Current commands include:

```text
ovrin-ns --root /namespace move --map MOVE_IN.tsv --journal MOVE_JOURNAL.tsv
ovrin-ns switch --manifest /namespace/ovrin.build.json --set SWITCH.tsv --before /holds/run --journal /holds/run.tsv --dry-run
ovrin-ns --root /namespace check --class core --changed-since /state/stamp --json
```

`ovrin-ns resolve [--manifest M] [--journal J]... [--json] [--stdin] PATH...` answers where an old path is now, for any reader (tracker, Lorengine, the walk, an agent), so none carries its own table. A path that exists is printed unchanged; otherwise the manifest's `retired_roots` and the move journals are tried, following a chain of moves to its end. The journals are the manifest's `move_ledger` file plus its `journals` globs (relative to the ledger's role; when absent, the 19 Oct `MOVE_JOURNAL*.tsv` and the 28 Oct cleanup's `mapping/*.journal.tsv`), then each `--journal`; `.inverse.tsv` files and rows that are neither `applied` nor `pending` are skipped. Each line of output is `<path now>` TAB `<how>` (`exists`, `root`, `journal <file>:<line>`, joined by ` > ` along a chain) or `not found` TAB `<path asked>`; `--json` prints one array of `{input, path, how}`, `path` null when not found; `--stdin` reads one path per line. Without `--manifest` or `--root` it uses the nearest `ovrin.build.json`, then `$HOME/ovrin.build.json`. Exit 0 when every path resolved, 1 when any did not, 2 on a bad manifest or journal argument. It is read only: no file is opened for writing, no link is made, nothing is started.

When an input table path or row path is absent beneath a retired root, a selected manifest can resolve it to its existing successor. Present original paths take precedence. Null successors and paths absent under both names are refused when the operation requires an existing source. WRITE output names the original and successor, and every affected journal act carries `resolved_from`. Free inverse destinations and output journal/hold paths use their existing successor parent; no inverse overwrites an occupied path. Use `--root` or the move/switch `--manifest` option to select the map. Switch rows with an empty staged field or a reason starting `HOLD` reject the entire run before any act.

`registry/allow/*.toml` under the governance role supplies counted historical allowances. Each `[[allow]]` has namespace-relative `file`, exact catalogue `entry`, integer `count` and a written `reason`. Accepted counts are subtracted per file and entry and emitted separately as `allowed` in JSON and counted text output. Missing reasons or files, constitution targets and unsafe paths are findings. Overdrawn counts are stale and suppress nothing. Only Chronicles carry lineage inside the constitution; its CHANGELOG.md is a fixed record and is refused.

`--changed-since` uses the stamp's modification time and checks only later modified governed documents in the selected class. With no path arguments, it selects the manifest's existing roots. Core means constitution, governance outside skills, and README.md/AGENTS.md/DIRECTORY_MAP.md front doors; other living prose is spine. Full and timestamp scans prune record containers and the manifest’s excluded payloads. They never read old document contents just to test the timestamp. Invalid allowances still report their own defects; stale counts are evaluated only for files actually checked.


The namespace contains its living material; fix before-copies stay outside it. `fix --write` requires `--before /outside/living/text/hold` in addition to its accepted plan and ledger. The tool resolves the hold’s existing ancestors, rejects a symlink into living text, preserves prior bytes and permissions there, and journals the actual backup path. It refuses an internal living-text hold before creating any output. A manifest-excluded declared record hold is permitted after migration; its journal must be beside the hold.

Root paths resolve in both directions through `retired_roots`, preserving any existing original. This lookup serves move, switch and read-only preflight. Printed resolutions retain `resolved_from`; mutation journals retain it per act and preflight exposes it in JSON. Staged preflight reads the consumer table at its declared staging source, checks prepared files, and validates planned link targets and key sources without opening native history folders. Its sibling `MOVE_IN.tsv` supplies only the places arriving at move-in: matching missing structural/landing/link paths are counted under INFO P9; unrelated missing paths still fail. The map itself grants no write permission. This is a readiness check, not a claim that those records have already arrived.

Whole-namespace checks need no path list. Both full and timestamp-filtered checks read the manifest's `index.roles` and `index.exclusions` at runtime: role-relative, case-sensitive globs, `except_paths`, and role-scoped exceptions apply before `never > list_only > embed`. Unmatched files stay out. Directories route to eligible descendants unless a declaration excludes them; excluded directories are never descended and links are never followed. With an index policy, core/spine membership is identical for full and timestamp-filtered checks. Legacy manifests without an index retain explicit-path behavior. Refused records remain individually visible and do not change check status. The catalogue and allowances are still loaded for each invocation.

## Canon colour

`ovrin-canon` puts the canon rules of `ovrin_ns::canon` to work. The canon is the manifest's roots and the subtrees it declares: projects, their documentation, landing places, the stratum's members, and the folders on the way to them. A declared folder that carries a root number below the root level is an exception. Content nested in a root is never foreign.

```text
ovrin-canon check [--json]
ovrin-canon colour --dry-run
ovrin-canon colour --apply --record /new/colour-applied.tsv
ovrin-canon colour --undo /path/colour-applied.tsv
```

`check` lists the top-level entries of the home root and of the stratum that the manifest does not know, or that go against its convention. It exits with 1 when it lists any. `colour` gives every canon folder one colour, yellow, through the file manager's per-folder picture metadata (`gio`). It also clears that colour from any folder that is no longer canon. An apply refuses an existing record, and it writes and syncs each folder's earlier values to the record before changing that folder. An undo restores those values exactly, and it refuses to restore anything if a folder changed after the apply. Running the apply again is idempotent: it changes nothing that is already right.
