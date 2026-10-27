# so-trust

Rate how far a Stack Overflow answer can be trusted **today**, before you paste its code.

Stack Overflow answers age and nobody maintains them. `so-trust` reads the public Stack Exchange API, checks each answer for age, removed APIs, "this is outdated" comments and version mismatch with your project, and returns `STALE`, `CHECK` or `OK` with the reasons and a source line.

One Python file, standard library only. Works as a command line tool or as an agent skill (`SKILL.md`).

## Why

- Stack Overflow is now a frozen archive. Questions created per month that are still visible through the API: 105,743 in Oct 2022, 52,334 in Oct 2023, 23,162 in Oct 2024, 5,263 in Oct 2025, 966 in Sep 2026 (measured 2026-10-27, deleted questions are not counted).
- Old answers stay on top. In the accepted answers of the 100 most-voted `pandas` questions (91 had one), the median age is 13 years and 85% were not edited in the last 3 years. For `tensorflow` (79 answers) it is 9.7 years and 94%, and 21 of the 79 use TensorFlow 1.x-only API such as `tf.Session`. The API check is a regex, so it shows a pattern, not proof that an answer is wrong.
- Published research agrees: more than half of obsolete answers (58.4%) were probably obsolete when first posted, and only 20.5% were ever updated ([Zhang et al., IEEE TSE 2019](https://arxiv.org/abs/1903.12282)).
- Plain HTTP clients get `403` with a Cloudflare challenge from stackoverflow.com. The API does not.

## Install

Needs Python 3 (tested on 3.14). No packages to install.

```bash
git clone <this repository>
cd so-trust
python so_trust.py --selftest
```

As an agent skill, copy the folder so that `SKILL.md` and `so_trust.py` stay together, for example to `~/.claude/skills/so-trust/`.

## Use

```bash
# search: assess the best answer of each of the top 5 matching questions
python so_trust.py "AttributeError: 'DataFrame' object has no attribute 'ix'" --tag pandas -n 5 --pkg pandas=2.2.0

# judge one link: stackoverflow.com/a/13842286
python so_trust.py --answer 13842286 --pkg pandas=2.2.0
```

Example output:

```
[STALE] Set value for particular cell in pandas DataFrame using index
  answer score 955, accepted, posted 2012-12-12, last edit 2017-09-16
  - uses removed API: pandas .ix/iteritems/as_matrix/get_value/set_value/DataFrame.append (found `.set_value(`)
  - posted 14y ago, untouched 9y
  code:
    df.xs('C')['x']=10
  source: https://stackoverflow.com/a/13842286 - Stack Overflow, CC BY-SA 4.0, by unutbu
```

Pass `--pkg NAME=VERSION` (repeatable) for the libraries involved; without it the version check is skipped.

| Verdict | Meaning |
|---|---|
| `STALE` | Uses a removed API, was written for an older major version than you run, or two or more comments say it is outdated. Do not paste the code. |
| `CHECK` | Posted over 5 years ago and not edited for 3, or one comment warns it is outdated. Verify against the official docs. |
| `OK` | No signal found. That is absence of evidence, so test it like any other change. |

## How it works

- Three API requests per search: questions, answers (accepted answer first, else most voted), and the 100 most-voted comments of those answers.
- Responses are cached for one hour in the system temp directory. The `backoff` field is honoured.
- Anonymous quota is 300 requests per day. Set `SE_API_KEY` to use an app key (10,000 per day by default, per the API docs).

## Terms and attribution

Read from the Stack Exchange pages on 2026-10-27, not legal advice:

- Only the public API is used, no scraping. Stack Overflow content is licensed CC BY-SA 4.0 and needs attribution: every result prints its source URL and author. Keep that line when you quote an answer.
- The API terms say nothing about LLM use or long-term caching. Do not bulk-harvest, redistribute or train on the output.
- This project is not affiliated with or endorsed by Stack Exchange, Inc.

## Limits

- The removed-API list (`DEAD_API` in `so_trust.py`) is small and hand-made: pandas, TensorFlow 1.x, Python 2, legacy React, Node.
- A removed-API match in an answer that only discusses the API is a false positive. Code blocks that quote a traceback are skipped; prose is not. The `found` fragment in the output shows what matched.
- Warning comments come from the 100 most-voted comments across the picked answers, so a quiet answer can lose its warnings.
- The version check compares major versions named in the answer text with `--pkg`. It misses answers that never name a version.
- A verdict is a heuristic, not proof that code works or fails.

## Contributing

The most useful pull request adds a removed API to `DEAD_API`: a regex and a short label.

```python
(r"componentWillMount|componentWillReceiveProps", "React legacy lifecycle"),
```

Add a case to `selftest()` for it and run `python so_trust.py --selftest`.

## License

MIT, see [LICENSE](LICENSE). Content returned by the Stack Exchange API keeps its own CC BY-SA 4.0 license.
