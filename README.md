# so-trust

Stack Overflow answers age and nobody maintains them. `so-trust` tells you, before you paste the code, whether an answer still works **for the versions your project runs**.

Run it inside your project folder. It reads your versions from the lockfile, `requirements.txt`, `package.json` or `pyproject.toml`, searches Stack Overflow through the public Stack Exchange API, and rates each answer `STALE`, `CHECK` or `OK` with the reasons and a source line. One Python file, standard library only, no account and no API key. Works as a command line tool or as an agent skill (`SKILL.md`).

## Why

- Stack Overflow is now a frozen archive. Questions created per month that are still visible through the API: 105,743 in Oct 2022, 52,334 in Oct 2023, 23,162 in Oct 2024, 5,263 in Oct 2025, 966 in Sep 2026 (measured 2026-10-27, deleted questions are not counted).
- Old answers stay on top. In the accepted answers of the 100 most-voted `pandas` questions (91 had one), the median age is 13 years and 85% were not edited in the last 3 years. For `tensorflow` (79 answers) it is 9.7 years and 94%, and 19 of the 79 use TensorFlow 1.x-only API such as `tf.Session` (checked by hand, see Evidence).
- Published research agrees: more than half of obsolete answers (58.4%) were probably obsolete when first posted, and only 20.5% were ever updated ([Zhang et al., IEEE TSE 2019](https://arxiv.org/abs/1903.12282)).
- Coding assistants hit this every day: [people ask](https://www.reddit.com/r/ChatGPTCoding/comments/1crznz2/why_do_all_of_the_llms_continue_to_generate/) why models keep writing outdated OpenAI API code.
- Plain HTTP clients get `403` with a Cloudflare challenge from stackoverflow.com. The API does not.

## Install

Needs Python 3 (tested on 3.14). No packages to install.

```bash
git clone https://github.com/Galactic717/so-trust
cd so-trust
python so_trust.py --selftest
```

With [uv](https://docs.astral.sh/uv/) you can also run it without cloning: `uvx --from git+https://github.com/Galactic717/so-trust so-trust --selftest`.

As an agent skill, copy the folder so that `SKILL.md` and `so_trust.py` stay together, for example to `~/.claude/skills/so-trust/`.

## Use

```bash
cd my-project   # so-trust reads requirements.txt, package.json, uv.lock, ... from here

# search: assess the best answer of each of the top 5 matching questions
python so_trust.py "AttributeError: 'DataFrame' object has no attribute 'ix'" --tag pandas

# judge one link: stackoverflow.com/a/13842286
python so_trust.py --answer 13842286
```

Example output for a project that pins `pandas==2.2.0`:

```
versions: pandas 2.2.0, python 3.14.5 (requirements.txt; python = running interpreter)

[STALE] Set value for particular cell in pandas DataFrame using index
  answer score 955, accepted, posted 2012-12-12, last edit 2017-09-16
  - uses as_matrix/get_value/set_value (found `.set_value(`), removed in pandas 1.0, you run 2.2.0
  - posted 14y ago, untouched 9y
  code:
    df.xs('C')['x']=10
  source: https://stackoverflow.com/a/13842286 - Stack Overflow, CC BY-SA 4.0, by unutbu
```

Versions come from lockfiles (`uv.lock`, `poetry.lock`, `package-lock.json`, exact), then `requirements*.txt` pins, then `package.json` / `pyproject.toml` (lower bound of the range). `python` is the interpreter running the script. Use `--project DIR` to read another folder and `--pkg NAME=VERSION` (repeatable) to override.

| Verdict | Meaning |
|---|---|
| `STALE` | Uses an API that was removed in a version at or below yours, or two or more comments say it is outdated. Do not paste the code. |
| `CHECK` | Posted over 5 years ago and not edited for 3, one comment warns it is outdated, the text names an older major version of a library you use, or it uses an API removed in some version and your version is unknown (pass `--pkg`). Verify against the official docs. |
| `OK` | No signal found. That is absence of evidence, so test it like any other change. |

Covered today: pandas, numpy, scikit-learn, TensorFlow 1.x, the pre-1.0 OpenAI SDK, React, and Python (standard library removals and Python 2 code). Each rule carries the release that removed the API, so an answer that is fine for your version is not flagged.

## Evidence

On 513 accepted answers from six tags, 53 were `STALE`. I read all 53: 52 were correct and 1 was a false positive. The first version of the rules scored worse (13 correct out of 18 reviewed), and the three fixes that followed are listed too. Recall is not measured. Method, caveats and a script to reproduce it: [evals/RESULTS.md](evals/RESULTS.md).

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

- The rule list (`RULES` in `so_trust.py`) is small and hand-made, so a removed API outside it goes unnoticed.
- A match next to words like "removed", "deprecated" or "instead of" is skipped because the answer is probably explaining the removal. That also hides real recommendations that use those words nearby.
- Version ranges in `package.json` and `pyproject.toml` are read as their lower bound, so a newer installed version can be missed. Lockfiles are exact.
- Warning comments come from the 100 most-voted comments across the picked answers, so a quiet answer can lose its warnings.
- A verdict is a heuristic, not proof that code works or fails.

## Contributing

The most useful pull request adds a removed API to `RULES`: a regex, the package, the first release without it, and a short label.

```python
(r"ReactDOM\.render\(", "react", "19.0", "ReactDOM.render"),
```

Add a case to `selftest()` and run `python so_trust.py --selftest`.

## License

MIT, see [LICENSE](LICENSE). Content returned by the Stack Exchange API keeps its own CC BY-SA 4.0 license.
