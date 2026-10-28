# Precision review

Date: 2026-10-27. Reproduce with `cargo run --release --example precision` (live data, so numbers can drift a little).

The review was done on the Python build (v0.2). The tool was then rewritten in Rust (v0.3): the Rust build gives the same totals (513 / 53 / 395 / 65) and printed output identical character for character on six inputs (three single answers, three searches).

## Setup

For six tags (`pandas`, `numpy`, `scikit-learn`, `tensorflow`, `reactjs`, `python`) take the accepted answers of the 100 most-voted questions: 513 answers. Pretend the project runs recent versions (pandas 2.2.0, numpy 2.0.0, scikit-learn 1.5.0, tensorflow 2.16.0, react 19.0.0, python 3.13.0) and ask so-trust for a verdict without comments. Then read every `STALE` verdict by hand.

A verdict counts as correct when the answer's text or code uses an API that was removed in the pinned version. It does not mean the answer is wrong for every reader.

## Result

| Verdict | Answers | Share |
|---|---|---|
| STALE | 53 | 10% |
| CHECK | 395 | 77% |
| OK | 65 | 13% |

By tag, STALE out of scanned: pandas 4/91, numpy 5/90, scikit-learn 5/76, tensorflow 19/79, reactjs 18/88, python 2/89.

Hand review of the 53 `STALE`: **52 correct, 1 false positive** ([a/30668609](https://stackoverflow.com/a/30668609) compares ES6 classes with `React.createClass` and does not recommend it). Two borderline cases counted as correct: [a/25254087](https://stackoverflow.com/a/25254087) retracts its own `.ix` suggestion for another reason but still shows it, and [a/47034889](https://stackoverflow.com/a/47034889) lists TensorFlow 1.x names inside a TF1-era answer.

## What the first pass got wrong

The first version of the rules had 5 false positives in 18 reviewed rule hits (13 correct). Three fixes came out of it:

- `df\w*.append(` matched `dfs.append(...)`, a plain `list.append`. Now only `df.append(` and `DataFrame.append(`.
- Answers that explain a removal ("np.asscalar was removed with 1.23", "use tf.compat.v1.Session() instead of tf.Session()") were flagged. A match next to words like removed, deprecated, instead of, moved is now skipped.
- "Written for numpy 1.x" from a version in the answer text caused most of the numpy `STALE` verdicts, and a version in prose is often history. It now only gives `CHECK`.

## Caveats

- One reviewer, the author. The sample is the most-voted answers, not a random one.
- Recall is not measured. Answers that use a removed API outside the rule list stay `OK` or `CHECK`.
- `CHECK` is broad on old popular tags: most of it is the "posted over 5 years ago, untouched for 3" rule.
- Comments were not fetched in this run, so the "two outdated comments" path is not covered.

## Second review: error-message searches (2026-10-28)

The first review used how-to style answers. An agent mostly searches with the text of an error, so I ran 10 real error messages, 3 results each: five failures reproduced on Python 3.14.5 (`Mapping` from `collections`, `getargspec`, `encodestring`, `time.clock`, `asyncio.coroutine`) and five typical errors for a project pinning pandas 2.2.2, numpy 2.0.1, scikit-learn 1.5.1, TensorFlow 2.16.1 and openai 1.40.0. Plus 3 how-to searches (pandas append, React render, OpenAI chat completion).

Before the fix, the error searches gave 6 `STALE`: **1 correct, 5 false.** The correct one is [a/77292230](https://stackoverflow.com/a/77292230), an accepted answer that tells an openai 1.40 user to call `openai.ChatCompletion.create`. The false ones explain the replacement or quote it: [a/76020727](https://stackoverflow.com/a/76020727) (a reference title), [a/74861894](https://stackoverflow.com/a/74861894) ("a direct replacement with np.int_"), [a/79338861](https://stackoverflow.com/a/79338861) ("change np.float to np.float64"), [a/74345066](https://stackoverflow.com/a/74345066) and [a/74343999](https://stackoverflow.com/a/74343999) (explaining the removal of `asyncio.coroutine`). The how-to searches gave 5 `STALE`, all 5 correct (`df.append`, `ReactDOM.render` twice, `openai.Completion.create`, `openai.ChatCompletion.create`).

Fixes:

- In a thread whose title is an error message (`no attribute`, `cannot import`, `No module named`, `ImportError`, `AttributeError`, `is not a function` ...), a match counts only inside a code block. The explanation check still looks at the prose around it. A first version that searched code blocks in isolation brought back one false positive, which the unchanged totals below would have hidden.
- `distutils` is no longer flagged. It left the standard library in Python 3.12 but `import distutils` still works on Python 3.14.5 when setuptools is installed (the file resolves to `setuptools/_distutils`).
- The `imp` rule missed `import imp` as the first line of a code block, because `^` does not match right after `<code>`. A new test caught it.

After the fixes: the error searches give 1 `STALE` (a/77292230), the how-to searches the same 5, and the 513-answer review above is unchanged (53 `STALE`, the same answers). Regression tests: offline cases for each pattern, and a live test on the six answer ids (`cargo test -- --ignored`).

Caveats: 10 error searches is a small sample. The `load_boston` search found only one unrelated thread, because ranking is Stack Exchange's. `CHECK` still marks old but correct fixes: in the error searches, 5 of the 8 `CHECK` results were the right answer.
