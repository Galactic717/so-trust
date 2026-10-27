#!/usr/bin/env python3
"""so-trust: how far can a Stack Overflow answer be trusted today?

Uses only the public Stack Exchange API (stdlib, no scraping). Content is CC BY-SA 4.0,
so every result prints its source URL and author. See SKILL.md for usage.
"""
import argparse, glob, gzip, hashlib, html, json, os, platform, re, sys, tempfile, time
import urllib.error, urllib.parse, urllib.request

try:
    import tomllib
except ImportError:  # Python < 3.11: pyproject.toml, uv.lock and poetry.lock are skipped
    tomllib = None

API = "https://api.stackexchange.com/2.3"
CACHE = os.path.join(tempfile.gettempdir(), "so-trust-cache")
TTL = 3600  # API docs: never repeat an identical request within a minute; answers rarely change
YEAR = 31557600
KEY = os.environ.get("SE_API_KEY")  # optional: lifts the anonymous 300 requests/day quota

# (regex, package, first release without it, label). A rule fires only when the question or
# answer mentions the package (NEEDLE), and flags STALE only if your version is >= the removal.
# ponytail: hand-curated list, coverage is the ceiling, extend via PR
RULES = [
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
    (r"sklearn\.externals\.(?:joblib|six)", "scikit-learn", "0.23", "sklearn.externals"),
    (r"\bload_boston\b|plot_(?:confusion_matrix|roc_curve|precision_recall_curve)\(", "scikit-learn", "1.2", "load_boston/plot_* helpers"),
    (r"tf\.(?:Session|placeholder|contrib|get_variable|app\.flags|global_variables_initializer|layers)", "tensorflow", "2.0", "TensorFlow 1.x API"),
    (r"openai\.(?:ChatCompletion|Completion|Embedding|Image)\.a?create", "openai", "1.0", "pre-1.0 module-level API"),
    (r"React\.(?:createClass|PropTypes)\b", "react", "16.0", "React.createClass/PropTypes"),
    (r"ReactDOM\.(?:render|hydrate)\(|unmountComponentAtNode|findDOMNode\(", "react", "19.0", "ReactDOM.render/findDOMNode"),
    (r"time\.clock\(|platform\.linux_distribution|cgi\.escape\(", "python", "3.8", "time.clock/linux_distribution/cgi.escape"),
    (r"base64\.(?:encodestring|decodestring)\(|\.isAlive\(", "python", "3.9", "base64.encodestring/Thread.isAlive"),
    (r"from collections import (?:Mapping|MutableMapping|Sequence|Iterable|Callable|Hashable)\b", "python", "3.10", "collections ABC aliases"),
    (r"inspect\.getargspec\(|asyncio\.coroutine\b", "python", "3.11", "inspect.getargspec/asyncio.coroutine"),
    (r"^\s*(?:import imp\b|from imp import|import distutils|from distutils)", "python", "3.12", "imp/distutils"),
    (r"\burllib2\b|\braw_input\(|\bxrange\(|\.has_key\(|\bprint\s+['\x22]|urllib\.(?:urlopen|urlencode|quote)\(", "python", "3.0", "Python 2-only code"),
]
NEEDLE = {"scikit-learn": r"sklearn|scikit", "tensorflow": r"tensorflow|\btf\.", "python": r"python|\bdef \w+\(|\bimport "}
EXPLAIN = re.compile(r"deprecat|removed|no longer|instead of|replaced|renamed|moved|was located|compat|however, now|used to", re.I)
WARN = re.compile(r"deprecated|no longer|any ?more|obsolete|outdated|was removed|been removed|removed in|not supported (?:in|any)", re.I)


def txt(s):
    return html.unescape(re.sub(r"<[^>]+>", "", s or ""))


def vt(v):
    return tuple(int(x) for x in re.findall(r"\d+", v)[:3])


def call(path, **params):
    params.setdefault("site", "stackoverflow")
    if KEY:
        params["key"] = KEY
    url = f"{API}/{path}?{urllib.parse.urlencode(sorted(params.items()))}"
    cached = os.path.join(CACHE, hashlib.sha1(url.encode()).hexdigest())
    if os.path.exists(cached) and time.time() - os.path.getmtime(cached) < TTL:
        with open(cached, encoding="utf-8") as f:
            return json.load(f)
    req = urllib.request.Request(url, headers={"Accept-Encoding": "gzip", "User-Agent": "so-trust"})
    try:
        r = urllib.request.urlopen(req, timeout=30)
    except urllib.error.HTTPError as e:  # the API reports errors as JSON with an HTTP error status
        r = e
    raw = r.read()
    data = json.loads(gzip.decompress(raw) if raw[:2] == b"\x1f\x8b" else raw)
    if "error_id" in data:
        sys.exit(f"Stack Exchange API error {data['error_id']}: {data.get('error_message')}")
    os.makedirs(CACHE, exist_ok=True)
    with open(cached, "w", encoding="utf-8") as f:
        json.dump(data, f)
    time.sleep(min(data.get("backoff", 0), 30))  # API contract: honour `backoff`
    return data


def project_versions(folder):
    """Versions of the packages named in RULES, read from lockfiles (exact), requirements pins, then manifests (lower bound)."""
    found, srcs = {}, []
    known = {r[1] for r in RULES}

    def put(name, spec, src):
        name = re.sub(r"[-_.]+", "-", name).lower()
        m = re.search(r"\d+(?:\.\d+)*", spec or "")
        if name in known and name != "python" and m and name not in found:  # python comes from the running interpreter
            found[name] = m.group()
            if src not in srcs:
                srcs.append(src)

    def path(name):
        p = os.path.join(folder, name)
        return p if os.path.isfile(p) else None

    for name in ("uv.lock", "poetry.lock"):
        if path(name) and tomllib:
            with open(path(name), "rb") as f:
                for pk in tomllib.load(f).get("package", []):
                    put(pk["name"], pk.get("version"), name)
    if path("package-lock.json"):
        with open(path("package-lock.json"), encoding="utf-8") as f:
            for k, pk in json.load(f).get("packages", {}).items():
                if k.startswith("node_modules/") and k.count("node_modules/") == 1:
                    put(k[13:], pk.get("version"), "package-lock.json")
    for p in sorted(glob.glob(os.path.join(folder, "requirements*.txt"))):
        with open(p, encoding="utf-8", errors="replace") as f:
            for line in f:
                m = re.match(r"\s*([A-Za-z0-9_.-]+)(?:\[[^\]]*\])?\s*==\s*([\w.]+)", line)
                if m:
                    put(m.group(1), m.group(2), os.path.basename(p))
    if path("package.json"):
        with open(path("package.json"), encoding="utf-8") as f:
            d = json.load(f)
        for sec in ("dependencies", "devDependencies"):
            for k, spec in d.get(sec, {}).items():
                put(k, spec, "package.json")
    if path("pyproject.toml") and tomllib:
        with open(path("pyproject.toml"), "rb") as f:
            d = tomllib.load(f)
        for dep in d.get("project", {}).get("dependencies", []):
            m = re.match(r"\s*([A-Za-z0-9_.-]+)(?:\[[^\]]*\])?\s*(.*)", dep)
            put(m.group(1), m.group(2), "pyproject.toml")
        for k, spec in d.get("tool", {}).get("poetry", {}).get("dependencies", {}).items():
            put(k, spec if isinstance(spec, str) else spec.get("version", ""), "pyproject.toml")
    return found, srcs


def assess(ans, comments, pkgs, ctx=""):
    """Return (verdict, reasons, warning comments) for an answer dict (filter=withbody).
    ctx is the question's tags and title; pkgs maps package name to the version you run."""
    now = time.time()
    tb = r"Traceback \(most recent|^\s*\w*Error:|-+&gt;\s*\d+"  # a quoted traceback names the API without recommending it
    scan = re.sub(r"<pre[^>]*>.*?</pre>", lambda m: "" if re.search(tb, m.group(), re.M) else m.group(), ans.get("body", ""), flags=re.S)
    text = html.unescape(scan)
    plain = re.sub(r"<[^>]+>", " ", text)
    mention = f"{ctx} {plain}"
    age = (now - ans["creation_date"]) / YEAR
    idle = (now - ans.get("last_edit_date", ans["creation_date"])) / YEAR
    red, yellow = [], []
    for rx, pkg, gone, label in RULES:
        # a match next to words like "removed" or "instead of" means the answer explains the removal, not recommends the API
        hit = next((m for m in re.finditer(rx, text, re.M) if not EXPLAIN.search(text[max(0, m.start() - 100):m.end() + 100])), None)
        if not hit or not re.search(NEEDLE.get(pkg, pkg), mention, re.I):
            continue
        what = f"uses {label} (found `{hit.group().strip()[:40]}`), removed in {pkg} {gone}"
        if pkg not in pkgs:
            yellow.append(f"{what}; pass --pkg {pkg}=VERSION to confirm")
        elif vt(pkgs[pkg]) >= vt(gone):
            red.append(f"{what}, you run {pkgs[pkg]}")
    for name, ver in pkgs.items():
        mine = re.match(r"\d+", ver)
        majors = [int(m) for m in re.findall(rf"\b{re.escape(name)}\b[^\n]{{0,20}}?\bv?(\d+)\.\d+", plain, re.I)]
        if mine and majors and max(majors) < int(mine.group()):
            yellow.append(f"names {name} {max(majors)}.x, you run {ver}")  # weak signal: a version in prose may be history, so CHECK only
    warn = [c for c in comments if WARN.search(txt(c.get("body")))]
    if warn:
        (red if len(warn) >= 2 else yellow).append(f"{len(warn)} comment(s) say it is outdated")
    if age > 5 and idle > 3:
        yellow.append(f"posted {age:.0f}y ago, untouched {idle:.0f}y")
    verdict = "STALE" if red else "CHECK" if yellow else "OK"
    return verdict, red + yellow, warn[:2]


def show(q, ans, comments, pkgs):
    verdict, why, warn = assess(ans, comments, pkgs, " ".join(q.get("tags", [])) + " " + q["title"])
    day = lambda t: time.strftime("%Y-%m-%d", time.gmtime(t))
    print(f"[{verdict}] {html.unescape(q['title'])}")
    print(f"  answer score {ans['score']}{', accepted' if ans.get('is_accepted') else ''}, "
          f"posted {day(ans['creation_date'])}, last edit {day(ans.get('last_edit_date', ans['creation_date']))}")
    for r in why:
        print(f"  - {r}")
    for c in warn:
        print(f"  > comment: {txt(c['body'])[:160]}")
    m = re.search(r"<pre[^>]*>\s*<code>(.*?)</code>", ans.get("body", ""), re.S)
    if m:
        print("  code:\n" + "\n".join("    " + l for l in html.unescape(m.group(1)).splitlines()[:12]))
    print(f"  source: https://stackoverflow.com/a/{ans['answer_id']} - Stack Overflow, CC BY-SA 4.0, "
          f"by {ans.get('owner', {}).get('display_name', 'unknown')}\n")


def selftest():
    now = time.time()
    a = lambda body, years=1: {"creation_date": now - years * YEAR, "body": body}
    pd, ix = {"pandas": "2.2.0"}, "<code>df.ix[1]</code>"
    assert assess(a(ix, 13), [], pd, "pandas")[0] == "STALE"
    assert assess(a(ix, 13), [], {"pandas": "0.25"}, "pandas")[0] == "CHECK"  # API still existed in your version, only old
    v, why, _ = assess(a(ix), [], {}, "pandas")
    assert v == "CHECK" and "--pkg pandas" in why[0], (v, why)  # version unknown: a hint, not a verdict
    assert assess(a("<code>x.ix[1]</code>"), [], pd, "javascript")[0] == "OK"  # not about pandas
    assert assess(a("<code>df.loc[1]</code>", 0.5), [], pd, "pandas")[0] == "OK"
    assert assess(a("<code>np.float(3)</code>"), [], {"numpy": "2.0.0"}, "numpy")[0] == "STALE"
    assert assess(a("<code>np.float64(3)</code>"), [], {"numpy": "2.0.0"}, "numpy")[0] == "OK"
    assert assess(a("<code>from collections import Mapping</code>"), [], {"python": "3.13.1"}, "python")[0] == "STALE"
    assert assess(a("x"), [{"body": "<p>This is deprecated now</p>"}], {})[0] == "CHECK"
    v, why, _ = assess(a("<p>Works on pandas 0.19 only</p>", 2), [], pd)
    assert v == "CHECK" and "pandas" in why[0], (v, why)
    assert assess(a("<code>dfs.append(data)</code>"), [], pd, "pandas")[0] == "OK"  # list.append, not DataFrame.append
    assert assess(a("<p>np.asscalar(x) was deprecated in 1.16 and removed in 1.23</p>"), [], {"numpy": "2.0.0"}, "numpy")[0] == "OK"
    assert assess(a("<p>use tf.compat.v1.Session() instead of tf.Session()</p>"), [], {"tensorflow": "2.16.0"}, "tensorflow")[0] == "OK"
    assert assess(a("<p>pandas 0.19 removed it, use pandas 2.1</p>", 2), [], pd)[0] == "OK"
    trace = "<pre><code>--&gt; 3 df.ix[1]\nAttributeError: 'DataFrame' object has no attribute 'ix'</code></pre>"
    assert assess(a(trace), [], pd, "pandas")[0] == "OK"  # quoted error, not a recommendation
    print("selftest ok")


def main():
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    ap = argparse.ArgumentParser(description="Rate how far Stack Overflow answers can be trusted today.")
    ap.add_argument("query", nargs="?", help="error message or question text")
    ap.add_argument("--answer", type=int, help="assess one answer id (the number in stackoverflow.com/a/ID)")
    ap.add_argument("--tag", help="restrict search to a tag, e.g. pandas")
    ap.add_argument("-n", type=int, default=5, help="questions to assess (default 5)")
    ap.add_argument("--project", default=".", help="folder with lockfile / requirements / package.json to read your versions from (default: .)")
    ap.add_argument("--pkg", action="append", default=[], metavar="NAME=VERSION", help="override a detected version, repeatable")
    ap.add_argument("--selftest", action="store_true", help="offline check of the verdict logic")
    a = ap.parse_args()
    if a.selftest:
        return selftest()
    if not (a.query or a.answer):
        ap.error("give a query or --answer ID")
    pkgs, srcs = project_versions(a.project)
    pkgs.update(dict(p.split("=", 1) for p in a.pkg))
    pkgs.setdefault("python", platform.python_version())
    print("versions: " + ", ".join(f"{k} {v}" for k, v in sorted(pkgs.items())) + f" ({', '.join(srcs) or 'no project files found'}; python = running interpreter)\n")
    if a.answer:
        ans = call(f"answers/{a.answer}", filter="withbody")["items"][0]
        q = call(f"questions/{ans['question_id']}")["items"][0]
        pairs = [(q, ans)]
    else:
        params = dict(q=a.query, sort="relevance", answers=1, pagesize=a.n)
        if a.tag:
            params["tagged"] = a.tag
        qs = call("search/advanced", **params)["items"]
        if not qs:
            sys.exit("no results")
        ids = ";".join(str(q["question_id"]) for q in qs)
        answers = call(f"questions/{ids}/answers", filter="withbody", sort="votes", pagesize=100)["items"]
        pairs = []
        for q in qs:
            mine = [x for x in answers if x["question_id"] == q["question_id"]]
            if mine:  # accepted answer if any, else the most voted (list is sorted by votes)
                pairs.append((q, next((x for x in mine if x.get("is_accepted")), mine[0])))
    # ponytail: one request for the 100 most-voted comments of all picked answers, a quiet answer can lose its warnings
    cm = call("answers/" + ";".join(str(x["answer_id"]) for _, x in pairs) + "/comments",
              filter="withbody", sort="votes", pagesize=100)["items"]
    for q, x in pairs:
        show(q, x, [c for c in cm if c["post_id"] == x["answer_id"]], pkgs)


if __name__ == "__main__":
    main()
