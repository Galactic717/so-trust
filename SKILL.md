---
name: so-trust
description: Use before applying any Stack Overflow answer, when the user pastes a stackoverflow.com link, or when an error message may be solved by a Stack Overflow thread. Rates answers STALE / CHECK / OK against the versions the project actually runs (read from its lockfile or manifest), using removed-API rules, age and "outdated" comments. Reads the public Stack Exchange API, so Cloudflare does not block it.
---

# so-trust

Stack Overflow answers age and nobody maintains them: about 3 in 5 obsolete answers were already obsolete when posted and only about 1 in 5 get updated (Zhang et al., TSE 2019). New questions have almost stopped, so the archive only gets older. This skill gives each answer a verdict for this project's versions before you paste its code.

## Run

`so_trust.py` sits next to this file. Python 3 (tested on 3.14), standard library only. Run it from the project root: it reads `uv.lock`, `poetry.lock`, `package-lock.json`, `requirements*.txt`, `package.json` and `pyproject.toml` there and prints the versions it found on the first line.

```bash
python so_trust.py "ImportError: cannot import name 'soft_unicode' from 'markupsafe'" --tag python
python so_trust.py --answer 13842286                  # judge one link: stackoverflow.com/a/13842286
python so_trust.py "query" --project path/to/app      # read versions from another folder
python so_trust.py "query" --pkg pandas=2.2.0         # override or add a version
python so_trust.py --selftest                         # offline check of the verdict logic
```

Check the `versions:` line. If a library the answer is about is missing, pass `--pkg NAME=VERSION` with the version from the project; names are `pandas`, `numpy`, `scikit-learn`, `tensorflow`, `openai`, `react`, `python`.

## Verdicts

- **STALE**: the answer uses an API removed in a version at or below the project's, or two or more comments say it is outdated. Do not paste the code. Take the idea, then check the current official docs.
- **CHECK**: old and untouched (posted over 5 years ago, not edited for 3), one comment warns it is outdated, the text names an older major version of a library the project uses, or it uses an API that some version removed and the project's version is unknown (the reason says `pass --pkg`). Verify against the docs before use.
- **OK**: no signal found. That is absence of evidence, so test the change like any other.

Each result prints the reasons, up to two warning comments, the first code block (12 lines) and a source line. When you quote an answer to the user, keep the source line: Stack Overflow content is CC BY-SA 4.0 and needs attribution.

## Rules for using the API

Read from the Stack Exchange terms on 2026-10-27:

- Use the API only. The network terms forbid copying site content outside the API for commercial use, and the API itself is exempt from that clause. Do not scrape stackoverflow.com; Cloudflare returns 403 to plain clients anyway.
- Attribution is required: text URLs are enough when there is no browser UI. The script prints them.
- Throttles: at most 30 requests per second per IP, honour the `backoff` field, never repeat an identical request within a minute. The script caches responses in the system temp directory for 1 hour.
- Quota: 300 requests per day anonymous (checked live), 10,000 with an app key. Set `SE_API_KEY` to use a key. One search costs 3 requests.
- The API terms say nothing about LLM use or persistent caching. Do not bulk-harvest, redistribute or train on the output. This is a reading of the public pages, not legal advice.

## Evidence and limits

- On 513 accepted answers from six tags, 53 were `STALE`; reading all 53 by hand found 52 correct and 1 false positive. Recall is not measured. See `evals/RESULTS.md` in the repository.
- The rule list `RULES` is small and hand-made (pandas, numpy, scikit-learn, TensorFlow 1.x, pre-1.0 OpenAI SDK, React, Python). A removed API outside it goes unnoticed.
- A match next to words like "removed" or "instead of" is skipped because the answer is probably explaining the removal.
- Version ranges in `package.json` and `pyproject.toml` count as their lower bound. Lockfiles are exact.
- Warning comments come from the 100 most-voted comments across all picked answers, so a quiet answer can lose its warnings.
- A verdict is a heuristic, not proof that code works or fails.
