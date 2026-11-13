use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

const PACKAGE_BINS: &[(&str, &str)] = &[
    ("subbit-canary", "canary"),
    ("subbit-gate", "subbit-gate"),
    ("subbit-index", "lineage-index"),
    ("subbit-claim", "subbit-claim"),
];

struct Built {
    profile: &'static str,
    binaries: Vec<(&'static str, PathBuf)>,
}

impl Built {
    fn canary(&self) -> &Path {
        &self.binaries[0].1
    }
}

fn main() -> Result<()> {
    let mut args = env::args().skip(1);
    let Some(command) = args.next() else {
        print_usage();
        return Ok(());
    };

    match command.as_str() {
        "build" => {
            let (release, target_dir) = parse_build_args(args)?;
            let built = build(release, target_dir.as_deref())?;
            print_built(&built);
            Ok(())
        }
        "canary" => {
            let (release, target_dir, canary_args) = parse_canary_args(args)?;
            let built = build(release, target_dir.as_deref())?;
            print_built(&built);
            run_canary(built.canary(), &canary_args)
        }
        "-h" | "--help" | "help" => {
            print_usage();
            Ok(())
        }
        other => bail!("unknown xtask command `{other}` (try `cargo xtask --help`)"),
    }
}

fn print_usage() {
    println!(
        "usage:
  cargo xtask build [--release] [--target-dir <dir>]
  cargo xtask canary [--release] [--target-dir <dir>] -- <canary args>"
    );
}

fn parse_build_args(mut args: impl Iterator<Item = String>) -> Result<(bool, Option<PathBuf>)> {
    let mut release = false;
    let mut target_dir = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--release" => release = true,
            "--target-dir" => {
                let dir = args.next().context("--target-dir requires a value")?;
                target_dir = Some(PathBuf::from(dir));
            }
            other => bail!("unknown build argument `{other}` (try `cargo xtask --help`)"),
        }
    }

    Ok((release, target_dir))
}

fn parse_canary_args(
    mut args: impl Iterator<Item = String>,
) -> Result<(bool, Option<PathBuf>, Vec<String>)> {
    let mut release = false;
    let mut target_dir = None;
    let mut canary_args = Vec::new();

    while let Some(arg) = args.next() {
        if arg == "--" {
            canary_args.extend(args);
            break;
        }

        match arg.as_str() {
            "--release" => release = true,
            "--target-dir" => {
                let dir = args.next().context("--target-dir requires a value")?;
                target_dir = Some(PathBuf::from(dir));
            }
            _ => {
                canary_args.push(arg);
                canary_args.extend(args);
                break;
            }
        }
    }

    Ok((release, target_dir, canary_args))
}

fn build(release: bool, target_dir_arg: Option<&Path>) -> Result<Built> {
    let root = workspace_root();
    let target_dir = target_directory(target_dir_arg, &root)?;

    let mut cmd = Command::new(cargo());
    cmd.current_dir(&root)
        .arg("build")
        .arg("--manifest-path")
        .arg(root.join("Cargo.toml"));

    if release {
        cmd.arg("--release");
    }

    if target_dir_arg.is_some() {
        cmd.arg("--target-dir").arg(&target_dir);
    }

    for (package, bin) in PACKAGE_BINS {
        cmd.arg("-p").arg(package).arg("--bin").arg(bin);
    }

    println!("cargo xtask: running `{}`", render(&cmd));
    let status = cmd
        .status()
        .with_context(|| format!("running `{}`", render(&cmd)))?;

    if !status.success() {
        bail!("`{}` failed with {status}", render(&cmd));
    }

    let profile = if release { "release" } else { "debug" };
    let dir = target_dir.join(profile);
    let binaries = PACKAGE_BINS
        .iter()
        .map(|(_, name)| {
            let file = format!("{name}{}", env::consts::EXE_SUFFIX);
            let path = dir.join(file);
            anyhow::ensure!(
                path.is_file(),
                "expected binary at {} after build",
                path.display()
            );
            Ok((*name, path))
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(Built { profile, binaries })
}

fn print_built(built: &Built) {
    let dir = built.binaries[0]
        .1
        .parent()
        .expect("binary paths always have a parent")
        .display();
    println!(
        "cargo xtask: built {} profile binaries in {}",
        built.profile, dir
    );
    for (name, path) in &built.binaries {
        println!("  {name}: {}", path.display());
    }
}

fn run_canary(bin: &Path, args: &[String]) -> Result<()> {
    let mut cmd = Command::new(bin);
    cmd.args(args);

    println!("cargo xtask: running `{}`", render(&cmd));

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        Err(cmd.exec()).with_context(|| format!("exec {}", bin.display()))
    }

    #[cfg(not(unix))]
    {
        let status = cmd
            .status()
            .with_context(|| format!("running {}", bin.display()))?;
        if !status.success() {
            std::process::exit(status.code().unwrap_or(1));
        }
        Ok(())
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask must live directly under the workspace root")
        .to_path_buf()
}

fn target_directory(explicit: Option<&Path>, root: &Path) -> Result<PathBuf> {
    if let Some(dir) = explicit {
        return if dir.is_absolute() {
            Ok(dir.to_path_buf())
        } else {
            Ok(env::current_dir()?.join(dir))
        };
    }

    let output = Command::new(cargo())
        .current_dir(root)
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--manifest-path",
        ])
        .arg(root.join("Cargo.toml"))
        .output()
        .context("running `cargo metadata`")?;

    if !output.status.success() {
        bail!(
            "`cargo metadata` failed with {}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let metadata: serde_json::Value =
        serde_json::from_slice(&output.stdout).context("parsing `cargo metadata` output")?;
    let dir = metadata
        .get("target_directory")
        .and_then(|value| value.as_str())
        .context("`cargo metadata` did not report target_directory")?;

    Ok(PathBuf::from(dir))
}

fn cargo() -> String {
    env::var("CARGO").unwrap_or_else(|_| "cargo".to_string())
}

fn render(cmd: &Command) -> String {
    let mut parts = vec![cmd.get_program().to_string_lossy().into_owned()];
    parts.extend(cmd.get_args().map(|arg| {
        let arg = arg.to_string_lossy();
        if arg.is_empty() || arg.contains(|c: char| c.is_whitespace() || c == '"' || c == '\'') {
            format!("{arg:?}")
        } else {
            arg.into_owned()
        }
    }));
    parts.join(" ")
}
