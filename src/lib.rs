//! so-trust: how far can a Stack Overflow answer be trusted today?
//!
//! Uses only the public Stack Exchange API (no scraping). Content is CC BY-SA 4.0, so every
//! result prints its source URL and author. See SKILL.md for usage.

use regex::{Captures, Regex, RegexBuilder};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

const API: &str = "https://api.stackexchange.com/2.3";
const TTL: u64 = 3600; // API docs: never repeat an identical request within a minute; answers rarely change
const YEAR: f64 = 31_557_600.0;

/// Package name -> version the project runs.
pub type Pkgs = BTreeMap<String, String>;

/// (regex, package, first release without it, label). A rule fires only when the question or
/// answer mentions the package, and flags STALE only if your version is at or above the removal.
// ponytail: hand-curated list, coverage is the ceiling, extend via PR
pub const RULES: &[(&str, &str, &str, &str)] = &[
    (r"\.ix\[", "pandas", "1.0", ".ix indexer"),
    (r"\bas_matrix\(|\.get_value\(|\.set_value\(", "pandas", "1.0", "as_matrix/get_value/set_value"),
    (r"\.iteritems\(", "pandas", "2.0", "iteritems"),
    (r"\bdf\d*\.append\(|DataFrame\.append\(", "pandas", "2.0", "DataFrame.append"),
    (r"pd\.Panel\b", "pandas", "0.25", "pd.Panel"),
    (r"error_bad_lines|warn_bad_lines|pd\.datetime\b", "pandas", "2.0", "error_bad_lines/pd.datetime"),
    (r"\bnp\.(?:float|int|bool|object|str|long|unicode)\b", "numpy", "1.24", "np.float/int/bool/object/str aliases"),
    (r"\bnp\.asscalar\(", "numpy", "1.23", "np.asscalar"),
    (r"\bnp\.(?:NaN|Inf|infty|float_|product|cumproduct|sometrue|alltrue)\b", "numpy", "2.0", "np.NaN/Inf/float_/product aliases"),
    (r"sklearn\.(?:cross_validation|grid_search|learning_curve)", "scikit-learn", "0.20", "sklearn.cross_validation/grid_search"),
    (r"sklearn\.externals(?:\.|\s+import\s+)(?:joblib|six)", "scikit-learn", "0.23", "sklearn.externals"),
    (r"\bload_boston\b|plot_(?:confusion_matrix|roc_curve|precision_recall_curve)\(", "scikit-learn", "1.2", "load_boston/plot_* helpers"),
    (r"tf\.(?:Session|placeholder|contrib|get_variable|app\.flags|global_variables_initializer|layers)", "tensorflow", "2.0", "TensorFlow 1.x API"),
    (r"openai\.(?:ChatCompletion|Completion|Embedding|Image)\.a?create", "openai", "1.0", "pre-1.0 module-level API"),
    (r"React\.(?:createClass|PropTypes)\b", "react", "16.0", "React.createClass/PropTypes"),
    (r"ReactDOM\.(?:render|hydrate)\(|unmountComponentAtNode|findDOMNode\(", "react", "19.0", "ReactDOM.render/findDOMNode"),
    (r"time\.clock\(|platform\.linux_distribution|cgi\.escape\(", "python", "3.8", "time.clock/linux_distribution/cgi.escape"),
    (r"base64\.(?:encodestring|decodestring)\(|\.isAlive\(", "python", "3.9", "base64.encodestring/Thread.isAlive"),
    (r"from collections import (?:Mapping|MutableMapping|Sequence|Iterable|Callable|Hashable)\b", "python", "3.10", "collections ABC aliases"),
    (r"inspect\.getargspec\(|asyncio\.coroutine\b", "python", "3.11", "inspect.getargspec/asyncio.coroutine"),
    // distutils is left out on purpose: gone from the stdlib in 3.12 but it still imports when setuptools is installed
    (r"(?:^|>)\s*(?:import imp\b|from imp import)", "python", "3.12", "imp module"), // `>`: first line of a <code> block
    (r"\burllib2\b|\braw_input\(|\bxrange\(|\.has_key\(|\bprint\s+['\x22]|urllib\.(?:urlopen|urlencode|quote)\(", "python", "3.0", "Python 2-only code"),
];

/// Regex the question or answer must mention before a rule of this package can fire.
fn needle(pkg: &str) -> &str {
    match pkg {
        "scikit-learn" => r"sklearn|scikit",
        "tensorflow" => r"tensorflow|\btf\.",
        "python" => r"python|\bdef \w+\(|\bimport ",
        p => p,
    }
}

macro_rules! re {
    ($p:expr) => {{
        static R: OnceLock<Regex> = OnceLock::new();
        R.get_or_init(|| Regex::new($p).unwrap())
    }};
}

struct Rule {
    re: Regex,
    needle: Regex,
    pkg: &'static str,
    gone: &'static str,
    label: &'static str,
}

fn rules() -> &'static [Rule] {
    static R: OnceLock<Vec<Rule>> = OnceLock::new();
    R.get_or_init(|| {
        RULES
            .iter()
            .map(|&(rx, pkg, gone, label)| Rule {
                re: RegexBuilder::new(rx).multi_line(true).build().expect(rx),
                needle: RegexBuilder::new(needle(pkg)).case_insensitive(true).build().unwrap(),
                pkg,
                gone,
                label,
            })
            .collect()
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Ok,
    Check,
    Stale,
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match self {
            Level::Ok => "OK",
            Level::Check => "CHECK",
            Level::Stale => "STALE",
        })
    }
}

pub struct Verdict {
    pub level: Level,
    pub reasons: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Answer {
    pub id: u64,
    pub question_id: u64,
    pub score: i64,
    pub accepted: bool,
    pub created: i64,
    pub edited: Option<i64>,
    pub body: String,
    pub author: String,
}

#[derive(Clone, Debug)]
pub struct Question {
    pub id: u64,
    pub title: String,
    pub tags: Vec<String>,
}

fn decode(s: &str) -> String {
    html_escape::decode_html_entities(s).into_owned()
}

/// Decoded text of an HTML body, plus the byte ranges of it that came from `<pre>` code blocks.
fn decode_ranges(html: &str) -> (String, Vec<std::ops::Range<usize>>) {
    let (mut out, mut ranges, mut last) = (String::new(), Vec::new(), 0);
    for m in re!(r"(?s)<pre[^>]*>.*?</pre>").find_iter(html) {
        out += &decode(&html[last..m.start()]);
        let start = out.len();
        out += &decode(m.as_str());
        ranges.push(start..out.len());
        last = m.end();
    }
    out += &decode(&html[last..]);
    (out, ranges)
}

impl Answer {
    pub fn from_json(v: &Value) -> Answer {
        Answer {
            id: v["answer_id"].as_u64().unwrap_or(0),
            question_id: v["question_id"].as_u64().unwrap_or(0),
            score: v["score"].as_i64().unwrap_or(0),
            accepted: v["is_accepted"].as_bool().unwrap_or(false),
            created: v["creation_date"].as_i64().unwrap_or(0),
            edited: v["last_edit_date"].as_i64(),
            body: v["body"].as_str().unwrap_or("").to_string(),
            author: decode(v["owner"]["display_name"].as_str().unwrap_or("unknown")),
        }
    }
}

impl Question {
    pub fn from_json(v: &Value) -> Question {
        Question {
            id: v["question_id"].as_u64().unwrap_or(0),
            title: decode(v["title"].as_str().unwrap_or("")),
            tags: v["tags"].as_array().into_iter().flatten().filter_map(|t| t.as_str()).map(String::from).collect(),
        }
    }
}

/// Lower-case, with runs of `-_.` folded to `-`, as package indexes compare names.
pub fn norm(name: &str) -> String {
    re!(r"[-_.]+").replace_all(name, "-").to_lowercase()
}

/// First three numeric parts, zero padded, so "19" compares equal to "19.0".
fn vt(v: &str) -> [u32; 3] {
    let mut out = [0; 3];
    for (slot, m) in out.iter_mut().zip(re!(r"\d+").find_iter(v)) {
        *slot = m.as_str().parse().unwrap_or(0);
    }
    out
}

/// Plain text of an HTML fragment: tags become spaces, entities are decoded.
pub fn plain(html: &str) -> String {
    re!(r"<[^>]+>").replace_all(&decode(html), " ").into_owned()
}

fn txt(html: &str) -> String {
    decode(&re!(r"<[^>]+>").replace_all(html, ""))
}

/// `before` chars ahead of byte `s` through `after` chars past byte `e`.
pub fn window(t: &str, s: usize, e: usize, before: usize, after: usize) -> &str {
    let a = t[..s].char_indices().rev().take(before).last().map_or(s, |(i, _)| i);
    let b = t[e..].char_indices().nth(after).map_or(t.len(), |(i, _)| e + i);
    &t[a..b]
}

/// Rate an answer. `ctx` is the question's tags and title, `pkgs` the versions you run.
pub fn assess(ans: &Answer, comments: &[String], pkgs: &Pkgs, ctx: &str, now: i64) -> Verdict {
    // a quoted traceback names the API without recommending it
    let tb = re!(r"(?m)Traceback \(most recent|^\s*\w*Error:|-+&gt;\s*\d+");
    let scan = re!(r"(?s)<pre[^>]*>.*?</pre>").replace_all(&ans.body, |c: &Captures| {
        if tb.is_match(&c[0]) { String::new() } else { c[0].to_string() }
    });
    let (text, code) = decode_ranges(&scan);
    let flat = re!(r"<[^>]+>").replace_all(&text, " ");
    let mention = format!("{ctx} {flat}");
    // In a thread about an error message the answers explain the replacement in prose, so only matches inside code blocks count there.
    let error_thread = re!(r"(?i)no attribute|cannot import|no module named|importerror|attributeerror|modulenotfounderror|is not a function|is no longer supported").is_match(ctx);
    let counts = |at: usize| !error_thread || code.iter().any(|c| c.contains(&at));
    let age = (now - ans.created) as f64 / YEAR;
    let idle = (now - ans.edited.unwrap_or(ans.created)) as f64 / YEAR;
    let (mut red, mut yellow) = (Vec::<String>::new(), Vec::<String>::new());

    let explain = re!(r"(?i)deprecat|removed|no longer|instead of|replaced|renamed|moved|was located|compat|however, now|used to");
    for r in rules() {
        // a match next to words like "removed" or "instead of" means the answer explains the removal, not recommends the API
        let hit = r.re.find_iter(&text).find(|m| counts(m.start()) && !explain.is_match(window(&text, m.start(), m.end(), 100, 100)));
        let Some(m) = hit else { continue };
        if !r.needle.is_match(&mention) {
            continue;
        }
        let found: String = m.as_str().trim_start_matches('>').trim().chars().take(40).collect();
        let what = format!("uses {} (found `{}`), removed in {} {}", r.label, found, r.pkg, r.gone);
        match pkgs.get(r.pkg) {
            None => yellow.push(format!("{what}; pass --pkg {}=VERSION to confirm", r.pkg)),
            Some(v) if vt(v) >= vt(r.gone) => red.push(format!("{what}, you run {v}")),
            _ => {}
        }
    }
    for (name, ver) in pkgs {
        let Some(major) = re!(r"^\d+").find(ver).and_then(|m| m.as_str().parse::<u32>().ok()) else { continue };
        let named = Regex::new(&format!(r"(?i)\b{}\b[^\n]{{0,20}}?\bv?(\d+)\.\d+", regex::escape(name))).unwrap();
        let newest = named.captures_iter(&flat).filter_map(|c| c[1].parse::<u32>().ok()).max();
        if let Some(n) = newest.filter(|&n| n < major) {
            yellow.push(format!("names {name} {n}.x, you run {ver}")); // weak signal: a version in prose may be history, so CHECK only
        }
    }
    let warn: Vec<String> = comments
        .iter()
        .map(|c| txt(c))
        .filter(|c| re!(r"(?i)deprecated|no longer|any ?more|obsolete|outdated|was removed|been removed|removed in|not supported (?:in|any)").is_match(c))
        .collect();
    if !warn.is_empty() {
        let line = format!("{} comment(s) say it is outdated", warn.len());
        if warn.len() >= 2 { red.push(line) } else { yellow.push(line) }
    }
    if age > 5.0 && idle > 3.0 {
        yellow.push(format!("posted {age:.0}y ago, untouched {idle:.0}y"));
    }
    let level = if !red.is_empty() { Level::Stale } else if !yellow.is_empty() { Level::Check } else { Level::Ok };
    red.extend(yellow);
    Verdict { level, reasons: red, warnings: warn.into_iter().take(2).collect() }
}

/// `YYYY-MM-DD` of a Unix timestamp (UTC).
pub fn ymd(ts: i64) -> String {
    let z = ts.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:04}-{:02}-{:02}", yoe + era * 400 + i64::from(m <= 2), m, d)
}

pub fn render(q: &Question, a: &Answer, v: &Verdict) -> String {
    let mut s = format!(
        "[{}] {}\n  answer score {}{}, posted {}, last edit {}\n",
        v.level,
        q.title,
        a.score,
        if a.accepted { ", accepted" } else { "" },
        ymd(a.created),
        ymd(a.edited.unwrap_or(a.created))
    );
    for r in &v.reasons {
        s += &format!("  - {r}\n");
    }
    for c in &v.warnings {
        s += &format!("  > comment: {}\n", c.chars().take(160).collect::<String>());
    }
    if let Some(c) = re!(r"(?s)<pre[^>]*>\s*<code>(.*?)</code>").captures(&a.body) {
        s += "  code:\n";
        for l in decode(&c[1]).lines().take(12) {
            s += &format!("    {l}\n");
        }
    }
    s + &format!("  source: https://stackoverflow.com/a/{} - Stack Overflow, CC BY-SA 4.0, by {}\n", a.id, a.author)
}

fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
}

/// GET from the Stack Exchange API, cached for an hour in the system temp directory.
pub fn call(path: &str, params: &[(&str, &str)]) -> Result<Value, String> {
    let mut q: Vec<(String, String)> = params.iter().map(|&(k, v)| (k.to_string(), v.to_string())).collect();
    if !q.iter().any(|(k, _)| k == "site") {
        q.push(("site".into(), "stackoverflow".into()));
    }
    if let Ok(key) = std::env::var("SE_API_KEY") {
        q.push(("key".into(), key)); // optional: lifts the anonymous 300 requests/day quota
    }
    q.sort();
    let id = format!("{path}?{}", q.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&"));
    let cached = std::env::temp_dir().join("so-trust-cache").join(format!("{:016x}", fnv(&id)));
    let fresh = std::fs::metadata(&cached)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|d| d.as_secs() < TTL);
    if fresh {
        if let Some(v) = std::fs::read_to_string(&cached).ok().and_then(|s| serde_json::from_str(&s).ok()) {
            return Ok(v);
        }
    }
    let mut req = ureq::get(&format!("{API}/{path}")).set("User-Agent", "so-trust").timeout(Duration::from_secs(30));
    for (k, v) in &q {
        req = req.query(k, v);
    }
    let resp = match req.call() {
        Ok(r) => r,
        Err(ureq::Error::Status(_, r)) => r, // the API reports errors as JSON with an HTTP error status
        Err(e) => return Err(format!("request failed: {e}")),
    };
    let data: Value = serde_json::from_reader(resp.into_reader()).map_err(|e| format!("bad API response: {e}"))?;
    if let Some(id) = data.get("error_id") {
        return Err(format!("Stack Exchange API error {id}: {}", data["error_message"].as_str().unwrap_or("")));
    }
    if let Some(dir) = cached.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&cached, data.to_string());
    std::thread::sleep(Duration::from_secs(data["backoff"].as_u64().unwrap_or(0).min(30))); // API contract: honour `backoff`
    Ok(data)
}

/// One answer by id, with its question.
pub fn answer_pair(id: u64) -> Result<(Question, Answer), String> {
    let r = call(&format!("answers/{id}"), &[("filter", "withbody")])?;
    let a = Answer::from_json(r["items"].get(0).ok_or(format!("answer {id} not found"))?);
    let q = call(&format!("questions/{}", a.question_id), &[])?;
    let q = Question::from_json(q["items"].get(0).ok_or(format!("question {} not found", a.question_id))?);
    Ok((q, a))
}

/// Best answer (accepted, else most voted) of each question matching `query`.
pub fn search_pairs(query: &str, tag: Option<&str>, n: u32) -> Result<Vec<(Question, Answer)>, String> {
    let n = n.to_string();
    let mut params = vec![("q", query), ("sort", "relevance"), ("answers", "1"), ("pagesize", n.as_str())];
    if let Some(t) = tag {
        params.push(("tagged", t));
    }
    let r = call("search/advanced", &params)?;
    let qs: Vec<Question> = r["items"].as_array().into_iter().flatten().map(Question::from_json).collect();
    if qs.is_empty() {
        return Err("no results".into());
    }
    let ids = qs.iter().map(|q| q.id.to_string()).collect::<Vec<_>>().join(";");
    let r = call(&format!("questions/{ids}/answers"), &[("filter", "withbody"), ("sort", "votes"), ("pagesize", "100")])?;
    let answers: Vec<Answer> = r["items"].as_array().into_iter().flatten().map(Answer::from_json).collect();
    Ok(qs
        .into_iter()
        .filter_map(|q| {
            let mine: Vec<&Answer> = answers.iter().filter(|a| a.question_id == q.id).collect();
            let best = mine.iter().find(|a| a.accepted).or(mine.first()).map(|a| (*a).clone())?;
            Some((q, best))
        })
        .collect())
}

/// The 100 most-voted comments of the picked answers, grouped by answer id.
// ponytail: one request for all answers, a quiet answer can lose its warnings
pub fn comments_for(pairs: &[(Question, Answer)]) -> Result<BTreeMap<u64, Vec<String>>, String> {
    let ids = pairs.iter().map(|(_, a)| a.id.to_string()).collect::<Vec<_>>().join(";");
    let r = call(&format!("answers/{ids}/comments"), &[("filter", "withbody"), ("sort", "votes"), ("pagesize", "100")])?;
    let mut out: BTreeMap<u64, Vec<String>> = BTreeMap::new();
    for c in r["items"].as_array().into_iter().flatten() {
        out.entry(c["post_id"].as_u64().unwrap_or(0)).or_default().push(c["body"].as_str().unwrap_or("").to_string());
    }
    Ok(out)
}

/// Version printed by `python --version`, or None when no Python is on PATH.
pub fn python_version() -> Option<String> {
    ["python", "python3", "py"].iter().find_map(|exe| {
        let out = std::process::Command::new(exe).arg("--version").output().ok()?;
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        re!(r"Python (\d+\.\d+(?:\.\d+)?)").captures(&text).map(|c| c[1].to_string())
    })
}

/// Versions of the packages named in `RULES`, read from lockfiles (exact), requirements pins,
/// then manifests (lower bound of a range). Unreadable files are reported on stderr and skipped.
pub fn project_versions(dir: &Path) -> (Pkgs, Vec<String>) {
    let known: HashSet<&str> = RULES.iter().map(|r| r.1).collect();
    let (mut found, mut srcs) = (Pkgs::new(), Vec::<String>::new());
    let mut put = |name: &str, spec: &str, src: &str| {
        let name = norm(name);
        if name == "python" || !known.contains(name.as_str()) || found.contains_key(&name) {
            return; // python comes from `python --version`
        }
        if let Some(m) = re!(r"\d+(?:\.\d+)*").find(spec) {
            found.insert(name, m.as_str().to_string());
            if !srcs.iter().any(|s| s == src) {
                srcs.push(src.to_string());
            }
        }
    };
    let read = |name: &str| -> Option<String> {
        let path = dir.join(name);
        if !path.is_file() {
            return None;
        }
        match std::fs::read(&path) {
            Ok(b) => Some(String::from_utf8_lossy(&b).into_owned()),
            Err(e) => {
                eprintln!("so-trust: cannot read {name}: {e}");
                None
            }
        }
    };
    for name in ["uv.lock", "poetry.lock"] {
        let Some(t) = read(name) else { continue };
        match t.parse::<toml::Table>() {
            Ok(doc) => {
                for pk in doc.get("package").and_then(|p| p.as_array()).into_iter().flatten() {
                    if let (Some(n), Some(v)) = (pk.get("name").and_then(|x| x.as_str()), pk.get("version").and_then(|x| x.as_str())) {
                        put(n, v, name);
                    }
                }
            }
            Err(e) => eprintln!("so-trust: cannot parse {name}: {e}"),
        }
    }
    if let Some(t) = read("package-lock.json") {
        match serde_json::from_str::<Value>(&t) {
            Ok(doc) => {
                for (k, pk) in doc["packages"].as_object().into_iter().flatten() {
                    if let Some(n) = k.strip_prefix("node_modules/").filter(|n| !n.contains("node_modules/")) {
                        put(n, pk["version"].as_str().unwrap_or(""), "package-lock.json");
                    }
                }
            }
            Err(e) => eprintln!("so-trust: cannot parse package-lock.json: {e}"),
        }
    }
    let mut reqs: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.starts_with("requirements") && n.ends_with(".txt"))
        .collect();
    reqs.sort();
    for name in reqs {
        for line in read(&name).unwrap_or_default().lines() {
            if let Some(c) = re!(r"^\s*([A-Za-z0-9_.-]+)(?:\[[^\]]*\])?\s*==\s*([\w.]+)").captures(line) {
                put(&c[1], &c[2], &name);
            }
        }
    }
    if let Some(t) = read("package.json") {
        match serde_json::from_str::<Value>(&t) {
            Ok(doc) => {
                for sec in ["dependencies", "devDependencies"] {
                    for (k, spec) in doc[sec].as_object().into_iter().flatten() {
                        put(k, spec.as_str().unwrap_or(""), "package.json");
                    }
                }
            }
            Err(e) => eprintln!("so-trust: cannot parse package.json: {e}"),
        }
    }
    if let Some(t) = read("pyproject.toml") {
        match t.parse::<toml::Table>() {
            Ok(doc) => {
                let deps = doc.get("project").and_then(|p| p.get("dependencies")).and_then(|d| d.as_array());
                for dep in deps.into_iter().flatten().filter_map(|d| d.as_str()) {
                    if let Some(c) = re!(r"^\s*([A-Za-z0-9_.-]+)(?:\[[^\]]*\])?\s*(.*)").captures(dep) {
                        put(&c[1], &c[2], "pyproject.toml");
                    }
                }
                let poetry = doc.get("tool").and_then(|t| t.get("poetry")).and_then(|p| p.get("dependencies")).and_then(|d| d.as_table());
                for (k, spec) in poetry.into_iter().flatten() {
                    let s = spec.as_str().or_else(|| spec.get("version").and_then(|v| v.as_str())).unwrap_or("");
                    put(k, s, "pyproject.toml");
                }
            }
            Err(e) => eprintln!("so-trust: cannot parse pyproject.toml: {e}"),
        }
    }
    (found, srcs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const NOW: i64 = 1_800_000_000;

    fn ans(body: &str, years: f64) -> Answer {
        Answer { id: 1, question_id: 1, score: 1, accepted: false, created: NOW - (years * YEAR) as i64, edited: None, body: body.into(), author: "t".into() }
    }

    fn pk(items: &[(&str, &str)]) -> Pkgs {
        items.iter().map(|&(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn level(body: &str, years: f64, comments: &[&str], pkgs: &[(&str, &str)], ctx: &str) -> Level {
        verdict(body, years, comments, pkgs, ctx).level
    }

    fn verdict(body: &str, years: f64, comments: &[&str], pkgs: &[(&str, &str)], ctx: &str) -> Verdict {
        let comments: Vec<String> = comments.iter().map(|c| c.to_string()).collect();
        assess(&ans(body, years), &comments, &pk(pkgs), ctx, NOW)
    }

    #[test]
    fn verdict_logic() {
        let pd = [("pandas", "2.2.0")];
        let ix = "<code>df.ix[1]</code>";
        assert_eq!(level(ix, 13.0, &[], &pd, "pandas"), Level::Stale);
        // the API still existed in your version, the answer is only old
        assert_eq!(level(ix, 13.0, &[], &[("pandas", "0.25")], "pandas"), Level::Check);
        // version unknown: a hint, not a verdict
        let v = verdict(ix, 1.0, &[], &[], "pandas");
        assert!(v.level == Level::Check && v.reasons[0].contains("--pkg pandas"), "{:?}", v.reasons);
        assert_eq!(level("<code>x.ix[1]</code>", 1.0, &[], &pd, "javascript"), Level::Ok); // not about pandas
        assert_eq!(level("<code>df.loc[1]</code>", 0.5, &[], &pd, "pandas"), Level::Ok);
        assert_eq!(level("x", 1.0, &["<p>This is deprecated now</p>"], &[], ""), Level::Check);
        let two = ["deprecated in 2.0", "no longer works"];
        assert_eq!(level("x", 1.0, &two, &[], ""), Level::Stale);
        assert_eq!(level("x", 6.0, &[], &[], ""), Level::Check); // old, untouched
        let v = verdict("<p>Works on pandas 0.19 only</p>", 2.0, &[], &pd, "");
        assert!(v.level == Level::Check && v.reasons[0].contains("pandas"), "{:?}", v.reasons);
        assert_eq!(level("<p>pandas 0.19 removed it, use pandas 2.1</p>", 2.0, &[], &pd, ""), Level::Ok);
    }

    #[test]
    fn quoted_errors_and_explanations_are_not_recommendations() {
        let pd = [("pandas", "2.2.0")];
        let trace = "<pre><code>--&gt; 3 df.ix[1]\nAttributeError: 'DataFrame' object has no attribute 'ix'</code></pre>";
        assert_eq!(level(trace, 1.0, &[], &pd, "pandas"), Level::Ok);
        let py_trace = "<pre><code>Traceback (most recent call last):\n  df.ix[1]\nKeyError</code></pre>";
        assert_eq!(level(py_trace, 1.0, &[], &pd, "pandas"), Level::Ok);
        assert_eq!(level("<code>dfs.append(data)</code>", 1.0, &[], &pd, "pandas"), Level::Ok); // list.append
        let np = [("numpy", "2.0.0")];
        assert_eq!(level("<p>np.asscalar(x) was deprecated in 1.16 and removed in 1.23</p>", 1.0, &[], &np, "numpy"), Level::Ok);
        let tf = [("tensorflow", "2.16.0")];
        assert_eq!(level("<p>use tf.compat.v1.Session() instead of tf.Session()</p>", 1.0, &[], &tf, "tensorflow"), Level::Ok);
        // a removal explained in one place does not hide a real use elsewhere
        let both = "<p>np.asscalar was removed.</p><p>Elsewhere</p><p>x = ".to_string() + &"a ".repeat(120) + "np.asscalar(y)</p>";
        assert_eq!(level(&both, 1.0, &[], &np, "numpy"), Level::Stale);
    }

    #[test]
    fn in_error_threads_only_code_blocks_count() {
        let np = [("numpy", "2.0.1")];
        let err = "AttributeError: module 'numpy' has no attribute 'float' numpy";
        // prose and inline code: the answer explains the replacement
        assert_eq!(level("<p>Change <code>np.float</code> to <code>np.float64</code> in your file.</p>", 1.0, &[], &np, err), Level::Ok);
        assert_eq!(level("<p>For np.int a direct replacement with np.int_ or int is also good.</p>", 1.0, &[], &np, err), Level::Ok);
        // the same prose in a how-to thread still counts
        assert_eq!(level("<p>Use <code>np.float(x)</code> to convert.</p>", 1.0, &[], &np, "numpy conversion"), Level::Stale);
        // a code block that recommends the removed API still counts in an error thread
        let code = "<p>Try this:</p><pre><code>completion = openai.ChatCompletion.create(model=\"gpt-4\")</code></pre>";
        let ctx = "AttributeError: module 'openai' has no attribute 'ChatCompletion' openai-api";
        assert_eq!(level(code, 1.0, &[], &[("openai", "1.40.0")], ctx), Level::Stale);
        // a reference title is not a recommendation
        let refs = "<pre><code>big_df = pd.concat(acc)</code></pre><p>References: <a href=\"x\">df.append() is not appending</a></p>";
        let ctx = "AttributeError: 'DataFrame' object has no attribute 'append' pandas";
        assert_eq!(level(refs, 1.0, &[], &[("pandas", "2.2.2")], ctx), Level::Ok);
    }

    #[test]
    fn distutils_is_not_flagged_but_imp_is() {
        let py = [("python", "3.14.5")];
        assert_eq!(level("<pre><code>from distutils.version import LooseVersion</code></pre>", 1.0, &[], &py, "python"), Level::Ok);
        assert_eq!(level("<pre><code>import imp</code></pre>", 1.0, &[], &py, "python"), Level::Stale);
    }

    /// (rule label, context the question would carry, snippets that must be STALE, snippets that must not)
    #[allow(clippy::type_complexity)]
    fn cases() -> Vec<(&'static str, &'static str, Vec<&'static str>, Vec<&'static str>)> {
        vec![
            (".ix indexer", "pandas", vec!["df.ix[1]", "df.ix[:, 'a']"], vec!["df.loc[1]", "df.iloc[1]"]),
            ("as_matrix/get_value/set_value", "pandas", vec!["df.as_matrix()", "df.get_value(0, 'a')", "df.set_value(0, 'a', 1)"], vec!["df.to_numpy()", "df.at[0, 'a']"]),
            ("iteritems", "pandas", vec!["for k, v in df.iteritems():"], vec!["for k, v in df.items():"]),
            ("DataFrame.append", "pandas", vec!["df = df.append(row)", "pd.DataFrame.append(a, b)", "df2.append(x)"], vec!["dfs.append(row)", "rows.append(row)", "df.loc[len(df)] = row"]),
            ("pd.Panel", "pandas", vec!["p = pd.Panel(data)"], vec!["p = pd.DataFrame(data)"]),
            ("error_bad_lines/pd.datetime", "pandas", vec!["pd.read_csv(f, error_bad_lines=False)", "pd.datetime.now()"], vec!["pd.read_csv(f, on_bad_lines='skip')"]),
            ("np.float/int/bool/object/str aliases", "numpy", vec!["np.float(1)", "np.int(2)", "np.bool(0)", "dtype=np.object"], vec!["np.float64(1)", "np.int_(2)", "np.bool_(0)", "np.floating"]),
            ("np.asscalar", "numpy", vec!["np.asscalar(a)"], vec!["a.item()"]),
            ("np.NaN/Inf/float_/product aliases", "numpy", vec!["x = np.NaN", "np.Inf", "np.float_(1)", "np.product(a)"], vec!["x = np.nan", "np.inf", "np.prod(a)"]),
            ("sklearn.cross_validation/grid_search", "scikit-learn", vec!["from sklearn.cross_validation import train_test_split", "from sklearn.grid_search import GridSearchCV"], vec!["from sklearn.model_selection import train_test_split"]),
            ("sklearn.externals", "scikit-learn", vec!["from sklearn.externals import joblib", "from sklearn.externals.joblib import dump", "import sklearn.externals.six"], vec!["import joblib"]),
            ("load_boston/plot_* helpers", "scikit-learn", vec!["load_boston()", "plot_confusion_matrix(clf, X, y)"], vec!["fetch_california_housing()", "ConfusionMatrixDisplay.from_estimator(clf, X, y)"]),
            ("TensorFlow 1.x API", "tensorflow", vec!["sess = tf.Session()", "x = tf.placeholder(tf.float32)", "tf.contrib.layers.fully_connected", "tf.get_variable('w')", "tf.global_variables_initializer()", "tf.layers.dense(x, 3)", "tf.app.flags.DEFINE_string"], vec!["tf.keras.layers.Dense(3)", "tf.compat.v1.Session()", "model = tf.keras.Sequential()"]),
            ("pre-1.0 module-level API", "openai-api", vec!["openai.ChatCompletion.create(model='gpt-4')", "openai.Completion.create(", "openai.Embedding.acreate(", "openai.Image.create("], vec!["client.chat.completions.create(model='gpt-4')", "openai.OpenAI()"]),
            ("React.createClass/PropTypes", "reactjs", vec!["var A = React.createClass({})", "A.propTypes = { x: React.PropTypes.string }"], vec!["class A extends React.Component {}", "import PropTypes from 'prop-types'"]),
            ("ReactDOM.render/findDOMNode", "reactjs", vec!["ReactDOM.render(<App />, el)", "ReactDOM.hydrate(<App />, el)", "ReactDOM.findDOMNode(this)", "unmountComponentAtNode(el)"], vec!["createRoot(el).render(<App />)", "hydrateRoot(el, <App />)"]),
            ("time.clock/linux_distribution/cgi.escape", "python", vec!["t = time.clock()", "platform.linux_distribution()", "cgi.escape(s)"], vec!["time.perf_counter()", "html.escape(s)"]),
            ("base64.encodestring/Thread.isAlive", "python", vec!["base64.encodestring(b)", "base64.decodestring(b)", "t.isAlive()"], vec!["base64.encodebytes(b)", "t.is_alive()"]),
            ("collections ABC aliases", "python", vec!["from collections import Mapping", "from collections import Iterable"], vec!["from collections.abc import Mapping", "from collections import OrderedDict"]),
            ("inspect.getargspec/asyncio.coroutine", "python", vec!["inspect.getargspec(f)", "@asyncio.coroutine"], vec!["inspect.getfullargspec(f)", "async def f(): pass"]),
            ("imp module", "python", vec!["import imp", "from imp import reload"], vec!["import importlib", "from importlib import reload", "from distutils.core import setup", "import distutils"]),
            ("Python 2-only code", "python", vec!["print 'hello'", "print \"hello\"", "for i in xrange(3):", "x = raw_input()", "import urllib2", "d.has_key(1)", "urllib.urlopen(u)", "urllib.urlencode(d)"], vec!["print('hello')", "for i in range(3):", "x = input()", "import urllib.request", "urllib.parse.urlencode(d)"]),
        ]
    }

    #[test]
    fn every_rule_flags_its_positives_and_spares_its_negatives() {
        let cases = cases();
        assert_eq!(cases.len(), RULES.len(), "one case per rule");
        for &(_, pkg, gone, label) in RULES {
            let (_, ctx, pos, neg) = cases.iter().find(|c| c.0 == label).unwrap_or_else(|| panic!("no test case for rule `{label}`"));
            for s in pos {
                assert_eq!(level(s, 1.0, &[], &[(pkg, gone)], ctx), Level::Stale, "`{s}` should be STALE for {pkg} {gone}");
                // the same code is fine for a project older than the removal
                assert_eq!(level(s, 1.0, &[], &[(pkg, "0.0.1")], ctx), Level::Ok, "`{s}` is fine for {pkg} 0.0.1");
                // and without a version it is only a hint
                assert_eq!(level(s, 1.0, &[], &[], ctx), Level::Check, "`{s}` without a version is CHECK");
            }
            for s in neg {
                assert_eq!(level(s, 1.0, &[], &[(pkg, gone)], ctx), Level::Ok, "`{s}` must not be flagged");
            }
        }
    }

    #[test]
    fn rules_need_the_package_to_be_mentioned() {
        let mut checked = 0;
        for (label, _, pos, _) in cases() {
            let (_, pkg, gone) = RULES.iter().find(|r| r.3 == label).map(|r| (r.0, r.1, r.2)).unwrap();
            let named = RegexBuilder::new(needle(pkg)).case_insensitive(true).build().unwrap();
            // snippets that do not name the package themselves, asked about something unrelated: no rule may fire
            for s in pos.iter().filter(|s| !named.is_match(s)) {
                assert_eq!(level(s, 1.0, &[], &[(pkg, gone)], "css"), Level::Ok, "`{s}` with unrelated context");
                checked += 1;
            }
        }
        assert!(checked > 20, "only {checked} snippets checked");
    }

    #[test]
    fn dates_and_versions() {
        assert_eq!(ymd(0), "1970-01-01");
        assert_eq!(ymd(1_600_000_000), "2020-09-13");
        assert_eq!(ymd(1_355_270_400), "2012-12-12");
        assert_eq!(ymd(951_782_400), "2000-02-29");
        assert!(vt("19") >= vt("19.0") && vt("2.2.0") >= vt("2.0") && vt("3.14.5") >= vt("3.12"));
        assert!(vt("1.5") < vt("2.0") && vt("0.25") < vt("1.0") && vt("0.0.1") < vt("0.20"));
        assert_eq!(norm("Scikit_Learn"), "scikit-learn");
    }

    #[test]
    fn window_respects_char_boundaries() {
        let t = "жжжжжABCжжжжж";
        let s = t.find('A').unwrap();
        assert_eq!(window(t, s, s + 3, 2, 2), "жжABCжж");
        assert_eq!(window("abc", 1, 2, 100, 100), "abc");
    }

    #[test]
    fn rendering_has_source_line_and_decoded_code() {
        let q = Question { id: 1, title: "How to &lt;x&gt;".into(), tags: vec![] };
        let mut a = ans("<pre><code>if a &lt; b:\n    pass</code></pre>", 1.0);
        a.id = 42;
        a.author = "Bob".into();
        let out = render(&q, &a, &assess(&a, &[], &Pkgs::new(), "", NOW));
        assert!(out.starts_with("[OK] How to &lt;x&gt;"), "{out}"); // titles are decoded when parsed, not when rendered
        assert!(out.contains("    if a < b:"), "{out}");
        assert!(out.ends_with("source: https://stackoverflow.com/a/42 - Stack Overflow, CC BY-SA 4.0, by Bob\n"), "{out}");
        let parsed = Question::from_json(&serde_json::json!({"question_id": 7, "title": "A &amp; B", "tags": ["x", "y"]}));
        assert_eq!((parsed.title.as_str(), parsed.tags.len()), ("A & B", 2));
    }

    fn fixture(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("so-trust-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (f, body) in files {
            std::fs::write(dir.join(f), body).unwrap();
        }
        dir
    }

    #[test]
    fn reads_requirements_pins() {
        let d = fixture("req", &[("requirements.txt", "# c\npandas==2.2.0\nnumpy[extra]==1.26.4\nScikit_Learn==1.5.0\nrequests>=2\n-r other.txt\nTensorFlow==2.16.1\n")]);
        let (p, s) = project_versions(&d);
        assert_eq!(p, pk(&[("numpy", "1.26.4"), ("pandas", "2.2.0"), ("scikit-learn", "1.5.0"), ("tensorflow", "2.16.1")]));
        assert_eq!(s, ["requirements.txt"]);
    }

    #[test]
    fn lockfile_beats_requirements_beats_manifest() {
        let d = fixture(
            "prec",
            &[
                ("uv.lock", "[[package]]\nname = \"pandas\"\nversion = \"2.1.4\"\n\n[[package]]\nname = \"other\"\nversion = \"9.9\"\n"),
                ("requirements.txt", "pandas==2.0.0\nnumpy==1.26.4\n"),
                ("pyproject.toml", "[project]\ndependencies = [\"pandas>=1.5\", \"numpy>=1.20\", \"openai>=1.0,<2\"]\n"),
            ],
        );
        let (p, s) = project_versions(&d);
        assert_eq!(p, pk(&[("numpy", "1.26.4"), ("openai", "1.0"), ("pandas", "2.1.4")]));
        assert_eq!(s, ["uv.lock", "requirements.txt", "pyproject.toml"]);
    }

    #[test]
    fn package_lock_is_exact_and_ignores_nested_copies() {
        let lock = r#"{"packages": {"": {}, "node_modules/react": {"version": "18.3.1"}, "node_modules/a/node_modules/react": {"version": "16.0.0"}}}"#;
        let d = fixture("npm", &[("package-lock.json", lock), ("package.json", r#"{"dependencies": {"react": "^17.0.0"}}"#)]);
        let (p, s) = project_versions(&d);
        assert_eq!(p, pk(&[("react", "18.3.1")]));
        assert_eq!(s, ["package-lock.json"]);
    }

    #[test]
    fn manifests_use_the_lower_bound() {
        let json = r#"{"dependencies": {"react": "^18.2.0", "openai": "~1.2.3", "left-pad": "latest"}, "devDependencies": {"numpy": "*"}}"#;
        let poetry = "[tool.poetry.dependencies]\npython = \"^3.10\"\nnumpy = \"^1.26\"\nscikit-learn = { version = \"1.5.0\", optional = true }\n";
        let (p, _) = project_versions(&fixture("json", &[("package.json", json)]));
        assert_eq!(p, pk(&[("openai", "1.2.3"), ("react", "18.2.0")]));
        let (p, _) = project_versions(&fixture("poetry", &[("pyproject.toml", poetry)]));
        assert_eq!(p, pk(&[("numpy", "1.26"), ("scikit-learn", "1.5.0")])); // python is never read from files
    }

    #[test]
    fn broken_or_missing_files_do_not_crash() {
        let d = fixture("bad", &[("package.json", "{not json"), ("pyproject.toml", "[[[ nope"), ("uv.lock", "x ="), ("requirements.txt", "pandas==2.2.0\n\u{0}\u{1}binary\n")]);
        let (p, _) = project_versions(&d);
        assert_eq!(p, pk(&[("pandas", "2.2.0")]));
        let (p, s) = project_versions(&std::env::temp_dir().join("so-trust-no-such-dir"));
        assert!(p.is_empty() && s.is_empty());
    }

    // network tests: cargo test -- --ignored
    #[test]
    #[ignore]
    fn live_known_answers() {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
        let (q, a) = answer_pair(13_842_286).unwrap();
        let v = assess(&a, &[], &pk(&[("pandas", "2.2.0")]), &q.tags.join(" "), now);
        assert_eq!(v.level, Level::Stale, "{:?}", v.reasons);
        assert!(a.author.len() > 1 && render(&q, &a, &v).contains("stackoverflow.com/a/13842286"));
        // an answer that only quotes the traceback
        let (q, a) = answer_pair(77_023_911).unwrap();
        let v = assess(&a, &[], &pk(&[("pandas", "2.2.0")]), &q.tags.join(" "), now);
        assert_eq!(v.level, Level::Ok, "{:?}", v.reasons);
        assert!(answer_pair(1).unwrap_err().contains("not found"));
        assert_eq!(search_pairs("zzzzqqxxnonexistentgibberish987", None, 3).unwrap_err(), "no results");
        let pairs = search_pairs("pandas drop duplicates", Some("pandas"), 2).unwrap();
        assert!(!pairs.is_empty() && comments_for(&pairs).is_ok());
    }

    /// Real answers from error-message searches (2026-10-28): the first five explain a replacement, the last one recommends the removed API.
    #[test]
    #[ignore]
    fn live_error_thread_regressions() {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
        let pins = pk(&[("pandas", "2.2.2"), ("numpy", "2.0.1"), ("openai", "1.40.0"), ("python", "3.14.5")]);
        for id in [76_020_727_u64, 74_861_894, 79_338_861, 74_345_066, 74_343_999] {
            let (q, a) = answer_pair(id).unwrap();
            let v = assess(&a, &[], &pins, &format!("{} {}", q.tags.join(" "), q.title), now);
            assert_ne!(v.level, Level::Stale, "a/{id} explains the replacement: {:?}", v.reasons);
        }
        let (q, a) = answer_pair(77_292_230).unwrap();
        let v = assess(&a, &[], &pins, &format!("{} {}", q.tags.join(" "), q.title), now);
        assert_eq!(v.level, Level::Stale, "a/77292230 recommends openai.ChatCompletion: {:?}", v.reasons);
    }
}
