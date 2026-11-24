use crate::{
    Error, Result, document, drift, fix, manifest::Namespace, migration, paths, preflight, releases,
};
use clap::{ArgGroup, Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;
#[derive(Parser)]
#[command(
    name = "ovrin-ns",
    version,
    about = "Check namespace prose, propose mechanical fixes and verify namespace structure"
)]
pub struct Cli {
    #[arg(long, global = true)]
    root: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Check {
        paths: Vec<PathBuf>,
        #[arg(long)]
        changed_since: Option<PathBuf>,
        #[arg(long, value_enum, default_value = "core")]
        class: Class,
        #[arg(long)]
        json: bool,
    },
    Fix(FixArgs),
    Move(MoveArgs),
    Switch(SwitchArgs),
    /// Print where each path is now, through the retired roots and the move journals
    Resolve(ResolveArgs),
    /// Say where the code behind each installed release is, for the project folder you are in (or a path)
    Source {
        /// A project folder or a path inside one; default: the current folder
        path: Option<PathBuf>,
        /// Every tool in the release index
        #[arg(long, conflicts_with = "path")]
        all: bool,
        #[arg(long)]
        manifest: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Compare the registry, the namespace law, the layout specification and the house rules with the disk, and
    /// every installed release with its project folder; print each drift or "no drift"
    Drift {
        #[arg(long)]
        manifest: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    Preflight {
        #[arg(long, value_enum, default_value = "pre")]
        mode: Mode,
        #[arg(long)]
        manifest: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
}
#[derive(Args)]
#[command(group(ArgGroup::new("source").required(true).args(["map", "inverse"])))]
struct MoveArgs {
    #[arg(long)]
    manifest: Option<PathBuf>,
    #[arg(long, requires = "journal")]
    map: Option<PathBuf>,
    #[arg(long, requires = "map")]
    journal: Option<PathBuf>,
    #[arg(long, conflicts_with = "dry_run")]
    inverse: Option<PathBuf>,
    #[arg(long)]
    dry_run: bool,
}
#[derive(Args)]
#[command(group(ArgGroup::new("source").required(true).args(["set", "inverse"])))]
struct SwitchArgs {
    #[arg(long)]
    manifest: Option<PathBuf>,
    #[arg(long, requires_all = ["journal", "before"])]
    set: Option<PathBuf>,
    #[arg(long, requires = "set")]
    before: Option<PathBuf>,
    #[arg(long, requires = "set")]
    journal: Option<PathBuf>,
    #[arg(long, conflicts_with = "dry_run")]
    inverse: Option<PathBuf>,
    #[arg(long)]
    dry_run: bool,
}
#[derive(Args)]
struct ResolveArgs {
    #[arg(long)]
    manifest: Option<PathBuf>,
    /// A further move journal (time, result, act), read after the manifest's
    #[arg(long = "journal")]
    journals: Vec<PathBuf>,
    #[arg(long)]
    json: bool,
    /// Read one path per line from standard input
    #[arg(long)]
    stdin: bool,
    #[arg(required_unless_present = "stdin")]
    paths: Vec<PathBuf>,
}
#[derive(Clone, ValueEnum)]
enum Class {
    Core,
    Spine,
}
#[derive(Clone, ValueEnum)]
enum Mode {
    Pre,
    Staged,
    Post,
}
#[derive(Args)]
#[command(group(ArgGroup::new("kind").required(true).args(["names","paths_mode"])),group(ArgGroup::new("action").required(true).args(["dry_run","write"])))]
struct FixArgs {
    #[arg(long)]
    names: bool,
    #[arg(long = "paths")]
    paths_mode: bool,
    #[arg(required = true)]
    targets: Vec<PathBuf>,
    #[arg(long)]
    dry_run: bool,
    #[arg(long, requires_all = ["accept","before"])]
    write: bool,
    #[arg(long)]
    accept: Option<PathBuf>,
    #[arg(long)]
    ledger: PathBuf,
    #[arg(long)]
    before: Option<PathBuf>,
}
pub fn run() -> Result<i32> {
    run_cli(Cli::parse())
}
pub fn run_cli(cli: Cli) -> Result<i32> {
    match cli.command {
        Command::Move(args) => {
            let ns = migration_namespace(cli.root.as_deref(), args.manifest.as_deref())?;
            let n = if let Some(path) = args.inverse {
                migration::inverse_with(&path, ns.as_ref())?
            } else {
                migration::move_map_with(
                    &args
                        .map
                        .ok_or_else(|| Error::Invalid("--map required".into()))?,
                    &args
                        .journal
                        .ok_or_else(|| Error::Invalid("--journal required".into()))?,
                    args.dry_run,
                    ns.as_ref(),
                )?
            };
            println!("{n} move items checked/completed");
            Ok(0)
        }
        Command::Switch(args) => {
            let ns = migration_namespace(cli.root.as_deref(), args.manifest.as_deref())?;
            let n = if let Some(path) = args.inverse {
                migration::inverse_with(&path, ns.as_ref())?
            } else {
                migration::switch_set_with(
                    &args
                        .set
                        .ok_or_else(|| Error::Invalid("--set required".into()))?,
                    &args
                        .before
                        .ok_or_else(|| Error::Invalid("--before required".into()))?,
                    &args
                        .journal
                        .ok_or_else(|| Error::Invalid("--journal required".into()))?,
                    args.dry_run,
                    ns.as_ref(),
                )?
            };
            println!("{n} switch items checked/completed");
            Ok(0)
        }
        Command::Resolve(args) => {
            let manifest = match args.manifest {
                Some(p) => p,
                None => Namespace::discover(cli.root.as_deref()).or_else(|e| {
                    std::env::var_os("HOME")
                        .map(|h| PathBuf::from(h).join("ovrin.build.json"))
                        .filter(|p| p.is_file())
                        .ok_or(e)
                })?,
            };
            let ns = Namespace::load(&manifest)?;
            let journals = paths::Journals::load(&ns, &args.journals)?;
            let mut inputs = args.paths;
            if args.stdin {
                for line in std::io::stdin().lines() {
                    let line = crate::io(std::path::Path::new("-"), line)?;
                    if !line.trim().is_empty() {
                        inputs.push(line.trim_end_matches(['\r', '\n']).into());
                    }
                }
            }
            let found: Vec<_> = inputs
                .iter()
                .map(|p| paths::resolve(&ns, &journals, p))
                .collect();
            if args.json {
                println!("{}", serde_json::to_string(&found)?);
            } else {
                for f in &found {
                    match &f.path {
                        Some(p) => println!("{}\t{}", p.display(), f.how.join(" > ")),
                        None => println!("not found\t{}", f.input.display()),
                    }
                }
            }
            Ok(i32::from(found.iter().any(|f| f.path.is_none())))
        }
        Command::Source {
            path,
            all,
            manifest,
            json,
        } => {
            let ns = Namespace::load(&home_manifest(cli.root.as_deref(), manifest)?)?;
            let index = releases::load(&ns)?;
            let here = if all {
                None
            } else {
                Some(crate::absolute(
                    path.as_deref().unwrap_or(std::path::Path::new(".")),
                )?)
            };
            let tools = releases::select(&ns, &index, here.as_deref());
            if tools.is_empty() {
                let at = here.map(|p| p.display().to_string()).unwrap_or_default();
                println!("no installed tool in the release index has its project folder at {at}");
                return Ok(1);
            }
            let found = tools
                .into_iter()
                .map(|t| releases::locate(&ns, t))
                .collect::<Result<Vec<_>>>()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&found)?);
            } else {
                for f in &found {
                    println!("{} (release {})", f.tool, f.release);
                    match &f.folder {
                        Some(p) => println!("  project folder  {}", p.display()),
                        None => println!("  project folder  none in the registry"),
                    }
                    println!(
                        "  recorded source {}",
                        if f.identity.is_empty() {
                            "none"
                        } else {
                            &f.identity
                        }
                    );
                    println!("  code            {}", f.state.words());
                    println!("  detail          {}", f.detail);
                    if let Some(s) = &f.see {
                        println!("  see it          {s}");
                    }
                    if let Some(n) = &f.note {
                        println!("  note            {n}");
                    }
                    println!("  kept by         {}", f.owner);
                }
            }
            Ok(i32::from(found.iter().any(|f| !f.state.in_folder())))
        }
        Command::Drift { manifest, json } => {
            let manifest = home_manifest(cli.root.as_deref(), manifest)?;
            let sections = drift::run(&manifest)?;
            let all: Vec<_> = sections.iter().flat_map(|s| &s.drift).collect();
            let listed = all.iter().filter(|d| d.listed.is_some()).count();
            if json {
                println!("{}", serde_json::to_string_pretty(&sections)?);
            } else {
                for s in &sections {
                    println!(
                        "{:<24} {:>5} checked  {} drift   ({})",
                        s.name,
                        s.checked,
                        s.drift.len(),
                        s.source
                    );
                }
                for s in &sections {
                    for d in &s.drift {
                        match &d.listed {
                            Some(l) => {
                                println!("DRIFT  {}\n       listed: {} ({})", d.key, l.owner, l.why)
                            }
                            None => println!("DRIFT  {}\n       not listed", d.key),
                        }
                    }
                }
                if all.is_empty() {
                    println!("no drift");
                } else {
                    println!(
                        "drift: {} ({listed} listed with an owner, {} not listed)",
                        all.len(),
                        all.len() - listed
                    );
                }
            }
            Ok(i32::from(!all.is_empty()))
        }
        Command::Preflight {
            mode,
            manifest,
            json,
        } => {
            let manifest = match manifest {
                Some(p) => p,
                None => Namespace::discover(cli.root.as_deref())?,
            };
            let mode = match mode {
                Mode::Pre => "pre",
                Mode::Staged => "staged",
                Mode::Post => "post",
            };
            let checks = preflight::run(&manifest, mode)?;
            if !json {
                println!(
                    "ovrin-ns {} mode={mode} manifest={}{}",
                    env!("CARGO_PKG_VERSION"),
                    manifest.display(),
                    if Namespace::load(&manifest)?.value["consumers"].is_null() {
                        " (v2; built-in consumer list)"
                    } else {
                        ""
                    }
                );
            }
            for c in &checks {
                if json {
                    println!("{}", serde_json::to_string(c)?)
                } else {
                    println!(
                        "{} {} {}  {} checked; {} findings",
                        c.status,
                        c.id,
                        c.name,
                        c.checked,
                        c.findings.len()
                    );
                    for r in &c.resolved_from {
                        println!(
                            "  INFO resolved_from={} -> {}",
                            r.resolved_from.display(),
                            r.path.display()
                        );
                    }
                    for f in &c.findings {
                        println!("  {f}");
                    }
                }
            }
            Ok(i32::from(checks.iter().any(|c| c.status == "FAIL")))
        }
        Command::Check {
            mut paths,
            class,
            json,
            changed_since,
        } => {
            let ns = Namespace::load(&Namespace::discover(cli.root.as_deref())?)?;
            // A relative path resolves against the working folder; a missing one is refused.
            paths = paths
                .into_iter()
                .map(|p| {
                    let a = crate::io(&p, std::path::absolute(&p))?;
                    crate::io(&a, std::fs::symlink_metadata(&a))?;
                    Ok(a)
                })
                .collect::<Result<_>>()?;
            let explicit = !paths.is_empty();
            if paths.is_empty() {
                paths = ns
                    .roots()
                    .into_iter()
                    .map(|(_, p)| p)
                    .filter(|p| p.is_dir())
                    .collect();
            }
            let report = document::inspect(
                &ns,
                &paths,
                match class {
                    Class::Core => "core",
                    Class::Spine => "spine",
                },
                changed_since.as_deref(),
            )?;
            if explicit
                && changed_since.is_none()
                && report.checked == 0
                && report.refused_records.is_empty()
                && report.skipped.is_empty()
                && report.findings.is_empty()
            {
                return Err(Error::Invalid(
                    "no governed document of this class under the given paths; 0 checked".into(),
                ));
            }
            if json {
                println!("{}", serde_json::to_string(&report)?);
            } else {
                for f in &report.findings {
                    println!(
                        "{}:{}: {}{}",
                        f.file.display(),
                        f.line,
                        f.entry,
                        f.detail
                            .as_ref()
                            .map(|s| format!(" ({s})"))
                            .unwrap_or_default()
                    );
                }
                eprintln!(
                    "{} documents checked; {} findings; {} records refused",
                    report.checked,
                    report.findings.len(),
                    report.refused_records.len()
                );
                eprintln!(
                    "{} occurrences allowed by {} counted allowances",
                    report.allowed.iter().map(|a| a.count).sum::<usize>(),
                    report.allowed.len()
                );
                for a in &report.allowed {
                    println!(
                        "allowed {} {} {}: {}",
                        a.count,
                        a.file.display(),
                        a.entry,
                        a.reason
                    );
                }
                for p in &report.refused_records {
                    eprintln!("refused record: {}", p.display());
                }
                for p in &report.skipped {
                    eprintln!("skipped, not UTF-8 text: {}", p.display());
                }
            }
            Ok(i32::from(!report.findings.is_empty()))
        }
        Command::Fix(args) => {
            let ns = Namespace::load(&Namespace::discover(cli.root.as_deref())?)?;
            let plan = fix::plan(&ns, &args.targets, args.names)?;
            if args.write {
                let accept = args
                    .accept
                    .ok_or_else(|| Error::Invalid("--write requires --accept".into()))?;
                let n = fix::apply(
                    &ns,
                    &plan,
                    &accept,
                    &args.ledger,
                    &args.before.ok_or_else(|| {
                        Error::Invalid("--write requires --before outside the namespace".into())
                    })?,
                )?;
                println!(
                    "{}",
                    serde_json::json!({"written":n,"skipped":plan.skipped})
                );
            } else {
                println!("{}", serde_json::to_string_pretty(&plan)?);
            }
            Ok(0)
        }
    }
}

/// The manifest named, else the nearest above here, else the home folder's.
fn home_manifest(root: Option<&std::path::Path>, manifest: Option<PathBuf>) -> Result<PathBuf> {
    match manifest {
        Some(p) => Ok(p),
        None => Namespace::discover(root).or_else(|e| {
            std::env::var_os("HOME")
                .map(|h| PathBuf::from(h).join("ovrin.build.json"))
                .filter(|p| p.is_file())
                .ok_or(e)
        }),
    }
}

fn migration_namespace(
    root: Option<&std::path::Path>,
    manifest: Option<&std::path::Path>,
) -> Result<Option<Namespace>> {
    if let Some(path) = manifest {
        return Namespace::load(path).map(Some);
    }
    if root.is_some() {
        return Namespace::load(&Namespace::discover(root)?).map(Some);
    }
    Ok(None)
}
