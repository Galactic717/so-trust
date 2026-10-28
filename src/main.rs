use so_trust::*;
use std::io::Write;
use std::path::Path;
use std::process::exit;
use std::time::{SystemTime, UNIX_EPOCH};

const HELP: &str = "so-trust - rate how far Stack Overflow answers can be trusted today

USAGE:
    so-trust <QUERY> [--tag TAG] [-n N] [--project DIR] [--pkg NAME=VERSION]...
    so-trust --answer ID [--project DIR] [--pkg NAME=VERSION]...

OPTIONS:
    --answer ID           assess one answer (the number in stackoverflow.com/a/ID)
    --tag TAG             restrict the search to a tag, e.g. pandas
    -n N                  questions to assess (default 5)
    --project DIR         folder with a lockfile / requirements / package.json to read versions from (default: .)
    --pkg NAME=VERSION    override or add a version, repeatable
    -h, --help            show this help
    -V, --version         show the version

Set SE_API_KEY to use a Stack Exchange app key (lifts the anonymous 300 requests/day quota).
";

/// Write to stdout and ignore errors: `so-trust ... | head` closes the pipe early and `println!` would panic.
fn out(s: &str) {
    let _ = std::io::stdout().write_all(s.as_bytes());
}

fn die(code: i32, msg: &str) -> ! {
    eprintln!("so-trust: {msg}");
    exit(code)
}

fn value(args: &mut impl Iterator<Item = String>, flag: &str) -> String {
    args.next().unwrap_or_else(|| die(2, &format!("{flag} needs a value")))
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut query, mut answer, mut tag, mut n, mut project) = (None::<String>, None::<u64>, None::<String>, 5u32, ".".to_string());
    let mut overrides: Vec<(String, String)> = Vec::new();
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => return out(HELP),
            "-V" | "--version" => return out(&format!("so-trust {}\n", env!("CARGO_PKG_VERSION"))),
            "--answer" => answer = Some(value(&mut args, &a).parse().unwrap_or_else(|_| die(2, "--answer needs a number"))),
            "--tag" => tag = Some(value(&mut args, &a)),
            "-n" => n = value(&mut args, &a).parse().unwrap_or_else(|_| die(2, "-n needs a number")),
            "--project" => project = value(&mut args, &a),
            "--pkg" => {
                let v = value(&mut args, &a);
                match v.split_once('=') {
                    Some((k, ver)) if !k.is_empty() && !ver.is_empty() => overrides.push((norm(k), ver.to_string())),
                    _ => die(2, &format!("--pkg needs NAME=VERSION, got `{v}`")),
                }
            }
            s if s.starts_with('-') && s.len() > 1 => die(2, &format!("unknown option `{s}`, see --help")),
            _ if query.is_none() => query = Some(a),
            _ => die(2, "only one query is accepted, quote it"),
        }
    }
    if query.as_deref().is_none_or(|q| q.trim().is_empty()) && answer.is_none() {
        die(2, "give a query or --answer ID, see --help");
    }

    let (mut pkgs, srcs) = project_versions(Path::new(&project));
    pkgs.extend(overrides);
    if !pkgs.contains_key("python") {
        if let Some(v) = python_version() {
            pkgs.insert("python".into(), v);
        }
    }
    let shown = pkgs.iter().map(|(k, v)| format!("{k} {v}")).collect::<Vec<_>>().join(", ");
    out(&format!(
        "versions: {} ({}; python = `python --version`)\n\n",
        if shown.is_empty() { "none".to_string() } else { shown },
        if srcs.is_empty() { "no project files found".to_string() } else { srcs.join(", ") }
    ));

    let pairs = match (answer, query) {
        (Some(id), _) => answer_pair(id).map(|p| vec![p]),
        (None, Some(q)) => search_pairs(&q, tag.as_deref(), n),
        _ => unreachable!(),
    }
    .unwrap_or_else(|e| die(1, &e));
    let comments = comments_for(&pairs).unwrap_or_else(|e| die(1, &e));
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    for (q, a) in &pairs {
        let mine = comments.get(&a.id).map(Vec::as_slice).unwrap_or(&[]);
        let ctx = format!("{} {}", q.tags.join(" "), q.title);
        out(&format!("{}\n", render(q, a, &assess(a, mine, &pkgs, &ctx, now))));
    }
}
