---
name: so-trust
description: Use before applying any Stack Overflow answer, when the user pastes a stackoverflow.com link, or when an error message may be solved by a Stack Overflow thread. Rates answers STALE / CHECK / OK by age, removed-API usage, "outdated" comments and version mismatch with the project's installed versions. Reads the public Stack Exchange API, so Cloudflare does not block it.
---

# so-trust

Stack Overflow answers age and nobody maintains them: about 3 in 5 obsolete answers were already obsolete when posted and only about 1 in 5 get updated (Zhang et al., TSE 2019). New questions have almost stopped, so the archive only gets older. This skill gives each answer a verdict before you paste its code.

## Run

`so_trust.py` sits next to this file. Python 3 (tested on 3.14), standard library only.

```bash
python so_trust.py "ImportError: cannot import name 'soft_unicode' from 'markupsafe'" --tag python -n 5
python so_trust.py --answer 13842286 --pkg pandas=2.2.0     # judge one link: stackoverflow.com/a/13842286
python so_trust.py --selftest                               # offline check of the verdict logic
```

Pass `--pkg NAME=VERSION` (repeatable) for the libraries involved. Read the versions from the project's lockfile or manifest first; without them the version check is skipped.

## Verdicts

- **STALE**: uses an API that no longer exists, was written for an older major version than the project runs, or two or more comments say it is outdated. Do not paste the code. Take the idea, then check current official docs.
- **CHECK**: old and untouched (posted over 5 years ago and not edited for 3), or one comment warns it is outdated. Verify against the docs before use.
- **OK**: no signal found. That is absence of evidence, so test the change like any other.

Each result prints the reasons, up to two warning comments, the first code block (12 lines) and a source line. When you quote an answer to the user, keep the source line: Stack Overflow content is CC BY-SA 4.0 and needs attribution.

## Rules for using the API

Read from the Stack Exchange terms on 2026-10-27:

- Use the API only. The network terms forbid copying site content outside the API for commercial use, and the API itself is exempt from that clause. Do not scrape stackoverflow.com; Cloudflare returns 403 to plain clients anyway.
- Attribution is required: text URLs are enough when there is no browser UI. The script prints them.
- Throttles: at most 30 requests per second per IP, honour the `backoff` field, never repeat an identical request within a minute. The script caches responses in the system temp directory for 1 hour.
- Quota: 300 requests per day anonymous (checked live), 10,000 with an app key. Set `SE_API_KEY` to use a key. One search costs 3 requests.
- The API terms say nothing about LLM use or persistent caching. Do not bulk-harvest, redistribute or train on the output. This is a reading of the public pages, not legal advice.

## Limits

- The removed-API list in `DEAD_API` is small and hand-made (pandas, TensorFlow 1.x, Python 2, React legacy, Node). Add entries by pull request.
- A removed-API match in an answer that only discusses the API is a false positive. Code blocks that quote a traceback are skipped; prose is not. The `found` fragment in the output shows what matched.
- Warning comments come from the 100 most-voted comments across all picked answers, so a quiet answer can lose its warnings.
- The version check compares major versions found in the answer text with `--pkg`; it misses answers that never name a version.
- A verdict is a heuristic, not proof that code works or fails.
