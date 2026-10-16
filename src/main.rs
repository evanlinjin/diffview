mod diff;
mod gh;
mod highlight;
mod lsp;
mod theme;
mod view;

use anyhow::Context;

const USAGE: &str = "usage: dv [base] [--pr N [-f]]
  dv            diff working tree vs merge-base of the PR's base (or origin/HEAD)
  dv main       ... vs merge-base of main
  dv --pr 123   gh pr checkout 123 first
  -f, --force   with --pr: reset the local PR branch to the PR head (drops its local commits)";

fn main() -> anyhow::Result<()> {
    let (mut base, mut pr_num, mut force) = (None, None, false);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            "--pr" => pr_num = Some(args.next().context("--pr needs a number")?.parse().context("--pr needs a number")?),
            "-f" | "--force" => force = true,
            _ if a.starts_with('-') => anyhow::bail!("unknown flag {a}\n{USAGE}"),
            _ => base = Some(a),
        }
    }
    let root = diff::git(&["rev-parse", "--show-toplevel"])?;
    std::env::set_current_dir(root.trim())?;
    if let Some(n) = pr_num {
        gh::checkout(n, force)?;
    }
    // Comments only load when the branch has a PR; plain local diffs still work without one.
    let pr = gh::pr(pr_num).ok();
    if let (Some(p), None) = (&pr, &base) {
        _ = diff::git(&["fetch", "-q", "origin", &p.base]);
        base = Some(format!("origin/{}", p.base));
    }
    let base = diff::merge_base(base.as_deref())?;
    let mut app = view::App::new(base, pr)?;
    let mut term = ratatui::init();
    let res = app.run(&mut term);
    ratatui::restore();
    res
}
