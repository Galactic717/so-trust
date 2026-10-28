//! Reproduce the precision review in evals/RESULTS.md.
//!
//! Takes the accepted answers of the 100 most-voted questions of six tags, pretends the project runs
//! recent library versions, and prints every STALE verdict with the text around the match so a human
//! can judge each one. Run: cargo run --release --example precision

use so_trust::*;
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() -> Result<(), String> {
    let pins: Pkgs = [("pandas", "2.2.0"), ("numpy", "2.0.0"), ("scikit-learn", "1.5.0"), ("tensorflow", "2.16.0"), ("react", "19.0.0"), ("openai", "1.50.0"), ("python", "3.13.0")]
        .iter()
        .map(|&(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|e| e.to_string())?.as_secs() as i64;
    let (mut total, mut stale, mut check) = (0, 0, 0);
    for tag in ["pandas", "numpy", "scikit-learn", "tensorflow", "reactjs", "python"] {
        let r = call("questions", &[("tagged", tag), ("sort", "votes"), ("order", "desc"), ("pagesize", "100")])?;
        let by_answer: BTreeMap<u64, Question> = r["items"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|q| Some((q["accepted_answer_id"].as_u64()?, Question::from_json(q))))
            .collect();
        let ids = by_answer.keys().map(u64::to_string).collect::<Vec<_>>().join(";");
        let r = call(&format!("answers/{ids}"), &[("filter", "withbody"), ("pagesize", "100")])?;
        let answers: Vec<Answer> = r["items"].as_array().into_iter().flatten().map(Answer::from_json).collect();
        let mut n_stale = 0;
        for a in &answers {
            let q = &by_answer[&a.id];
            let v = assess(a, &[], &pins, &format!("{} {}", q.tags.join(" "), q.title), now);
            total += 1;
            check += usize::from(v.level == Level::Check);
            if v.level != Level::Stale {
                continue;
            }
            stale += 1;
            n_stale += 1;
            let text = plain(&a.body);
            let found = v.reasons[0].split('`').nth(1).unwrap_or("?");
            let at = text.find(found).unwrap_or(0);
            let around = window(&text, at, at, 55, 45 + found.len()).split_whitespace().collect::<Vec<_>>().join(" ");
            println!("[{}] https://stackoverflow.com/a/{} `{}` ..{around}..", &tag[..4], a.id, found.chars().take(22).collect::<String>());
        }
        println!("== {tag}: {} accepted answers, STALE {n_stale}\n", answers.len());
    }
    println!("TOTAL scanned {total}, STALE {stale}, CHECK {check}, OK {}", total - stale - check);
    Ok(())
}
