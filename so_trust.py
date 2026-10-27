#!/usr/bin/env python3
"""so-trust: how far can a Stack Overflow answer be trusted today?

Uses only the public Stack Exchange API (stdlib, no scraping). Content is CC BY-SA 4.0,
so every result prints its source URL and author. See SKILL.md for usage.
"""
import argparse, gzip, hashlib, html, json, os, re, sys, tempfile, time
import urllib.error, urllib.parse, urllib.request

API = "https://api.stackexchange.com/2.3"
CACHE = os.path.join(tempfile.gettempdir(), "so-trust-cache")
TTL = 3600  # API docs: never repeat an identical request within a minute; answers rarely change
YEAR = 31557600
KEY = os.environ.get("SE_API_KEY")  # optional: lifts the anonymous 300 requests/day quota

# ponytail: hand-curated list of APIs removed from their library, coverage is the ceiling, extend via PR
DEAD_API = [
    (r"\.ix\[|iteritems\(|as_matrix\(|\.get_value\(|\.set_value\(|\bdf\w*\.append\(", "pandas .ix/iteritems/as_matrix/get_value/set_value/DataFrame.append"),
    (r"tf\.Session|tf\.placeholder|tf\.contrib|tf\.get_variable|tf\.app\.flags|tf\.global_variables_initializer", "TensorFlow 1.x-only API"),
    (r"\burllib2\b|\braw_input\(|\bxrange\(|\.has_key\(|\bprint\s+['\x22]", "Python 2-only syntax or module"),
    (r"React\.createClass|componentWillMount|componentWillReceiveProps", "React legacy lifecycle / createClass"),
    (r"require\(['\x22]request['\x22]\)|new Buffer\(", "Node: deprecated request package / new Buffer()"),
]
WARN = re.compile(r"deprecated|no longer|any ?more|obsolete|outdated|was removed|been removed|removed in|not supported (?:in|any)", re.I)


def txt(s):
    return html.unescape(re.sub(r"<[^>]+>", "", s or ""))


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


def assess(ans, comments, pkgs):
    """Return (verdict, reasons, warning comments) for an answer dict (filter=withbody)."""
    now = time.time()
    tb = r"Traceback \(most recent|^\s*\w*Error:|-+&gt;\s*\d+"  # a quoted traceback names the API without recommending it
    scan = re.sub(r"<pre[^>]*>.*?</pre>", lambda m: "" if re.search(tb, m.group(), re.M) else m.group(), ans.get("body", ""), flags=re.S)
    text = html.unescape(scan)
    age = (now - ans["creation_date"]) / YEAR
    idle = (now - ans.get("last_edit_date", ans["creation_date"])) / YEAR
    red, yellow = [], []
    for rx, label in DEAD_API:
        hit = re.search(rx, text)
        if hit:
            red.append(f"uses removed API: {label} (found `{hit.group().strip()}`)")
    plain = re.sub(r"<[^>]+>", " ", text)
    for name, ver in pkgs.items():
        mine = re.match(r"\d+", ver)
        majors = [int(m) for m in re.findall(rf"\b{re.escape(name)}\b[^\n]{{0,20}}?\bv?(\d+)\.\d+", plain, re.I)]
        if mine and majors and max(majors) < int(mine.group()):
            red.append(f"written for {name} {max(majors)}.x, you run {ver}")
    warn = [c for c in comments if WARN.search(txt(c.get("body")))]
    if warn:
        (red if len(warn) >= 2 else yellow).append(f"{len(warn)} comment(s) say it is outdated")
    if age > 5 and idle > 3:
        yellow.append(f"posted {age:.0f}y ago, untouched {idle:.0f}y")
    verdict = "STALE" if red else "CHECK" if yellow else "OK"
    return verdict, red + yellow, warn[:2]


def show(q, ans, comments, pkgs):
    verdict, why, warn = assess(ans, comments, pkgs)
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
    assert assess({"creation_date": now - 13 * YEAR, "body": "<code>df.ix[1]</code>"}, [], {})[0] == "STALE"
    assert assess({"creation_date": now - 0.5 * YEAR, "body": "<code>df.loc[1]</code>"}, [], {})[0] == "OK"
    assert assess({"creation_date": now - YEAR, "body": "x"}, [{"body": "<p>This is deprecated now</p>"}], {})[0] == "CHECK"
    v, why, _ = assess({"creation_date": now - 2 * YEAR, "body": "<p>Works on pandas 0.19 only</p>"}, [], {"pandas": "2.2.0"})
    assert v == "STALE" and "pandas" in why[0], (v, why)
    assert assess({"creation_date": now - 2 * YEAR, "body": "<p>pandas 0.19 removed it, use pandas 2.1</p>"}, [], {"pandas": "2.2.0"})[0] == "OK"
    trace = "<pre><code>--&gt; 3 df.ix[1]\nAttributeError: 'DataFrame' object has no attribute 'ix'</code></pre>"
    assert assess({"creation_date": now - YEAR, "body": trace}, [], {})[0] == "OK"  # quoted error, not a recommendation
    print("selftest ok")


def main():
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    ap = argparse.ArgumentParser(description="Rate how far Stack Overflow answers can be trusted today.")
    ap.add_argument("query", nargs="?", help="error message or question text")
    ap.add_argument("--answer", type=int, help="assess one answer id (the number in stackoverflow.com/a/ID)")
    ap.add_argument("--tag", help="restrict search to a tag, e.g. pandas")
    ap.add_argument("-n", type=int, default=5, help="questions to assess (default 5)")
    ap.add_argument("--pkg", action="append", default=[], metavar="NAME=VERSION", help="installed version, repeatable")
    ap.add_argument("--selftest", action="store_true", help="offline check of the verdict logic")
    a = ap.parse_args()
    if a.selftest:
        return selftest()
    if not (a.query or a.answer):
        ap.error("give a query or --answer ID")
    pkgs = dict(p.split("=", 1) for p in a.pkg)
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
