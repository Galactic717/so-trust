"""Reproduce the precision review in RESULTS.md.

Takes the accepted answers of the 100 most-voted questions of six tags, pretends the project runs
recent library versions, and prints every STALE verdict with the text around the match so a human
can judge each one. Run from the repo root: python evals/precision.py
"""
import html, os, re, sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
import so_trust as st

PINS = {"pandas": "2.2.0", "numpy": "2.0.0", "scikit-learn": "1.5.0", "tensorflow": "2.16.0",
        "react": "19.0.0", "openai": "1.50.0", "python": "3.13.0"}
TAGS = ["pandas", "numpy", "scikit-learn", "tensorflow", "reactjs", "python"]

total = stale = check = 0
for tag in TAGS:
    qs = st.call("questions", tagged=tag, sort="votes", order="desc", pagesize=100)["items"]
    byid = {q["accepted_answer_id"]: q for q in qs if "accepted_answer_id" in q}
    answers = st.call("answers/" + ";".join(map(str, byid)), filter="withbody", pagesize=100)["items"]
    n_stale = 0
    for a in answers:
        q = byid[a["answer_id"]]
        verdict, why, _ = st.assess(a, [], PINS, " ".join(q.get("tags", [])) + " " + q["title"])
        total += 1
        check += verdict == "CHECK"
        if verdict != "STALE":
            continue
        stale += 1
        n_stale += 1
        text = re.sub(r"<[^>]+>", " ", html.unescape(a["body"]))
        found = re.search(r"found `([^`]*)`", why[0])
        i = text.find(found.group(1)) if found else 0
        print(f"[{tag[:4]}] https://stackoverflow.com/a/{a['answer_id']} `{found.group(1)[:22] if found else '?'}` .."
              f"{' '.join(text[max(0, i - 55):i + 45].split())}..")
    print(f"== {tag}: {len(answers)} accepted answers, STALE {n_stale}\n")
print(f"TOTAL scanned {total}, STALE {stale}, CHECK {check}, OK {total - stale - check}")
