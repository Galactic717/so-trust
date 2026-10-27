# Precision review

Date: 2026-10-27. Reproduce with `python evals/precision.py` (live data, so numbers can drift a little).

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
