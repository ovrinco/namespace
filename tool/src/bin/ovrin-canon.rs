use clap::{ArgGroup, Args, Parser, Subcommand};
use ovrin_ns::{
    Error, Result,
    canon::{self, Canon},
    manifest::Namespace,
};
use std::path::PathBuf;
#[derive(Parser)]
#[command(
    name = "ovrin-canon",
    version,
    about = "List what the manifest does not know and give the canon folders their one colour"
)]
struct Cli {
    /// The namespace root (the folder holding ovrin.build.json); default: the nearest one above here, else the home folder.
    #[arg(long, global = true)]
    root: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// List the top-level entries of the namespace root and of the stratum that the manifest does not know.
    Check {
        #[arg(long)]
        json: bool,
    },
    /// Give every canon folder the canon colour, or take it off again.
    Colour(ColourArgs),
}
#[derive(Args)]
#[command(group(ArgGroup::new("mode").required(true).args(["dry_run", "apply", "undo"])))]
struct ColourArgs {
    #[arg(long)]
    dry_run: bool,
    #[arg(long, requires = "record")]
    apply: bool,
    /// The new undo record an apply writes (refused if it exists).
    #[arg(long)]
    record: Option<PathBuf>,
    /// Restore exactly what this record's apply changed.
    #[arg(long, value_name = "RECORD")]
    undo: Option<PathBuf>,
    #[arg(long, default_value = canon::ICON)]
    icon: String,
    #[arg(long)]
    json: bool,
}
fn load(root: Option<PathBuf>) -> Result<Canon> {
    let manifest = match root {
        Some(r) => Namespace::discover(Some(&r))?,
        None => Namespace::discover(None).or_else(|_| {
            let home = std::env::var_os("HOME").ok_or_else(|| Error::Invalid("no HOME".into()))?;
            Namespace::discover(Some(&PathBuf::from(home)))
        })?,
    };
    Canon::new(&Namespace::load(&manifest)?)
}
fn show(m: &canon::Meta, want: &canon::Meta) -> String {
    match m {
        [None, None] => "plain".into(),
        m if m == want => "yellow".into(),
        [i, n] => format!(
            "icon={} name={}",
            i.as_deref().unwrap_or("-"),
            n.as_deref().unwrap_or("-")
        ),
    }
}
fn run() -> Result<i32> {
    let cli = Cli::parse();
    let canon = load(cli.root)?;
    match cli.command {
        Command::Check { json } => {
            let found = canon.foreign()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&found)?);
            } else {
                for e in &found {
                    let listed = if e.host_listed {
                        "\thost list: left in place"
                    } else {
                        ""
                    };
                    println!(
                        "{}\t{}\t{}\t{}{listed}",
                        format!("{:?}", e.class).to_lowercase(),
                        e.kind,
                        e.path.display(),
                        e.reason
                    );
                }
                println!(
                    "{} entries unknown to the manifest or against its convention",
                    found.len()
                );
            }
            Ok(i32::from(!found.is_empty()))
        }
        Command::Colour(a) => {
            let changes = if let Some(record) = &a.undo {
                canon::undo(record)?
            } else {
                if !std::path::Path::new(&a.icon).is_file() {
                    return Err(Error::Invalid(format!(
                        "{}: colour picture missing",
                        a.icon
                    )));
                }
                let plan = canon::plan(&canon, &a.icon)?;
                if a.apply {
                    let record = a
                        .record
                        .as_deref()
                        .ok_or_else(|| Error::Invalid("--apply needs --record".into()))?;
                    canon::apply(&plan, record)?;
                }
                plan
            };
            let want = canon::coloured(&a.icon);
            if a.json {
                println!("{}", serde_json::to_string_pretty(&changes)?);
            } else {
                let verb = if a.dry_run { "would" } else { "did" };
                for c in &changes {
                    println!(
                        "{}\t{}\t{} -> {}",
                        c.action,
                        c.path.display(),
                        show(&c.before, &want),
                        show(&c.after, &want)
                    );
                }
                let n = changes.iter().filter(|c| c.action != "kept").count();
                println!("{verb} change {n} of {} folders", changes.len());
            }
            Ok(0)
        }
    }
}
fn main() {
    match run() {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!(
                "{}",
                serde_json::json!({"error":"namespace_error","detail":e.to_string()})
            );
            std::process::exit(2);
        }
    }
}
