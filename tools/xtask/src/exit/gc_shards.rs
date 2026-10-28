//! GC-stress shards (M3-P3-GC-SHARD): one GC-stress run split by path prefixes over several jobs.
//! Moved from m0_exit.rs (M4-P0). The one change: [`merge_gc_shards`] takes the tree root, not
//! the m0-exit context.
//!
//! `--gc-shard <k>/<n>` prints the prefixes of shard k of n; CI runs each shard as
//! `ep9-test262 exec ... <prefixes>` (a filtered run compares only the tests it ran, and its JSON
//! names them in `run_paths`). The split is
//! a function of the test262 checkout alone: units are path prefixes, split by their next
//! character until each holds at most 1/(4n) of the test files, packed by file count (a failing file costs about as much as a
//! passing one under GC-stress), largest first, into the shard with the fewest files. Item 6
//! computes the same split from this tree's checkout, puts each given JSON into the one shard
//! whose files are exactly the files it ran and skipped, and needs every shard exactly once.

use super::json::Json;
use super::report::{Cell, Status, cell};
use super::t262::count;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The CI job and step of a GC-stress shard (`.github/workflows/ci.yml`).
pub(crate) const GC_JOB: &str = "test262 GC-stress";
pub(crate) const GC_STEP: &str = "test262 exec, GC-stress + eager feedback";
/// The largest split that item 6 looks for.
pub(crate) const MAX_GC_SHARDS: usize = 32;

/// The test files of a test262 checkout, sorted, as the runner finds them
/// (tools/test262/src/suite.rs `discover`: every `*.js` under `test/<dir>/`, no `_FIXTURE`).
pub(crate) fn test262_files(t262: &Path) -> Result<Vec<String>, String> {
    fn walk(dir: &Path, rel: &str, out: &mut Vec<String>) -> Result<(), String> {
        let rd =
            std::fs::read_dir(dir).map_err(|e| format!("cannot list {}: {e}", dir.display()))?;
        for entry in rd {
            let entry = entry.map_err(|e| format!("cannot list {}: {e}", dir.display()))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let kind = entry
                .file_type()
                .map_err(|e| format!("{}: {e}", entry.path().display()))?;
            let child = format!("{rel}/{name}");
            if kind.is_dir() {
                walk(&entry.path(), &child, out)?;
            } else if kind.is_file() && name.ends_with(".js") && !name.contains("_FIXTURE") {
                out.push(child);
            }
        }
        Ok(())
    }
    let test = t262.join("test");
    let mut out = Vec::new();
    let rd = std::fs::read_dir(&test)
        .map_err(|e| format!("cannot list {} (no test262 checkout?): {e}", test.display()))?;
    for entry in rd {
        let entry = entry.map_err(|e| format!("cannot list {}: {e}", test.display()))?;
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            let name = entry.file_name().to_string_lossy().into_owned();
            walk(&entry.path(), &format!("test/{name}"), &mut out)?;
        }
    }
    out.sort();
    Ok(out)
}

/// The n shards of the split: each shard's prefixes (sorted) and its file count.
pub(crate) fn gc_split(files: &[String], n: usize) -> Result<Vec<(Vec<String>, usize)>, String> {
    if n == 0 || files.len() < n {
        return Err(format!("cannot split {} test files {n} ways", files.len()));
    }
    let limit = (files.len() / (4 * n)).max(1);
    // Units: (prefix, files). A prefix over the limit splits by the next character of its files'
    // paths: the children are disjoint and cover it, unless a file ends at the prefix itself.
    let mut todo = vec!["test/".to_string()];
    let mut units: Vec<(String, usize)> = Vec::new();
    while let Some(prefix) = todo.pop() {
        // Sorted, so the files under a prefix are one range.
        let lo = files.partition_point(|f| f.as_str() < prefix.as_str());
        let hi = lo + files[lo..].partition_point(|f| f.starts_with(&prefix));
        let under = &files[lo..hi];
        let children: BTreeSet<String> = under
            .iter()
            .filter_map(|f| f[prefix.len()..].chars().next())
            .map(|ch| format!("{prefix}{ch}"))
            .collect();
        if under.len() <= limit || under.iter().any(|f| f.len() == prefix.len()) {
            if !under.is_empty() {
                units.push((prefix, under.len()));
            }
        } else {
            todo.extend(children);
        }
    }
    units.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mut bins: Vec<(Vec<String>, usize)> = vec![(Vec::new(), 0); n];
    for (prefix, count) in units {
        let mut best = 0;
        for i in 1..n {
            if bins[i].1 < bins[best].1 {
                best = i;
            }
        }
        bins[best].0.push(prefix);
        bins[best].1 += count;
    }
    for (prefixes, count) in &mut bins {
        if *count == 0 {
            return Err(format!(
                "a split of {} test files {n} ways leaves a shard empty",
                files.len()
            ));
        }
        prefixes.sort();
    }
    Ok(bins)
}

/// `--gc-shard <k>/<n>`: the prefixes of shard k (1-based) of n.
pub(crate) fn gc_shard_prefixes(t262: &Path, spec: &str) -> Result<Vec<String>, String> {
    let parsed = spec
        .split_once('/')
        .and_then(|(k, n)| Some((k.parse::<usize>().ok()?, n.parse::<usize>().ok()?)));
    let Some((k, n)) = parsed.filter(|&(k, n)| (1..=n).contains(&k) && n <= MAX_GC_SHARDS) else {
        return Err(format!(
            "--gc-shard needs <k>/<n> with 1 <= k <= n <= {MAX_GC_SHARDS}, not {spec:?}"
        ));
    };
    let split = gc_split(&test262_files(t262)?, n)?;
    Ok(split[k - 1].0.clone())
}

/// The test files a filtered exec JSON selected (C-G1): its `run_paths` (every file it ran)
/// and its `skipped_paths`, sorted. Err if the JSON cannot say: no `run_paths` (a runner
/// before C-G1, or a run with no path filter), a list that disagrees with the file count, a
/// file both run and skipped, or a result for a file that was not run.
pub(crate) fn selected_paths(j: &Json) -> Result<Vec<&str>, String> {
    let strings =
        |key: &str| -> Option<Vec<&str>> { j.get(key)?.arr()?.iter().map(Json::str).collect() };
    let run = strings("run_paths").ok_or(
        "no run_paths, so the tests it ran cannot be checked (a runner before M3-P3 C-G1, or a run with no path filter)",
    )?;
    let skipped = strings("skipped_paths").ok_or("no skipped_paths")?;
    let total = count(j, &["files"]).map_or(0, |(_, t)| t);
    if run.len() as u64 != total {
        return Err(format!(
            "run_paths names {} files, \"files\" counts {total}",
            run.len()
        ));
    }
    let run_set: BTreeSet<&str> = run.iter().copied().collect();
    if let Some(p) = skipped.iter().find(|p| run_set.contains(*p)) {
        return Err(format!("{p} is both run and skipped"));
    }
    let results = j.get("results").and_then(Json::arr).unwrap_or(&[]);
    if let Some(p) = results
        .iter()
        .map(|r| r.get("path").and_then(Json::str).unwrap_or(""))
        .find(|p| !run_set.contains(p))
    {
        return Err(format!("a result for {p:?}, a file it did not run"));
    }
    let mut all: Vec<&str> = run.into_iter().chain(skipped).collect();
    all.sort_unstable();
    Ok(all)
}

/// The files of each shard of a split, sorted: a file is in the shard of the one prefix it
/// starts with (the prefixes of a split do not nest, so that prefix is the greatest one that
/// sorts at or before the file).
pub(crate) fn shard_members<'a>(
    files: &'a [String],
    split: &[(Vec<String>, usize)],
) -> Vec<Vec<&'a str>> {
    let mut prefixes: Vec<(&str, usize)> = split
        .iter()
        .enumerate()
        .flat_map(|(k, (pre, _))| pre.iter().map(move |p| (p.as_str(), k)))
        .collect();
    prefixes.sort_unstable();
    let mut members = vec![Vec::new(); split.len()];
    for f in files {
        let at = prefixes.partition_point(|(p, _)| *p <= f.as_str());
        if let Some(&(p, k)) = at.checked_sub(1).map(|i| &prefixes[i])
            && f.starts_with(p)
        {
            members[k].push(f.as_str());
        }
    }
    members
}

/// Several `--exec-gc` files: the shards of one split, merged into one whole-corpus JSON for
/// m0-exit's `gc_cell`. Err is the item 6 host cell (FAIL: a missing, extra or unplaced shard,
/// or shards of different runs).
pub(crate) fn merge_gc_shards(
    root: &Path,
    shards: &[(PathBuf, Json)],
) -> Result<(String, Json), Cell> {
    let fail = |why: String| {
        cell(
            Status::Fail,
            format!("{} GC-stress shard files: {why}", shards.len()),
        )
    };
    // One run: the same settings and expectation file in every shard.
    for key in [
        &["gc_stress"][..],
        &["timeout_ms"],
        &["feedback_threshold"],
        &["expectations", "file"],
    ] {
        let first = shards[0].1.path(key);
        if let Some((p, _)) = shards.iter().find(|(_, j)| j.path(key) != first) {
            return Err(fail(format!(
                "{} has another {} than {}",
                p.display(),
                key.join("."),
                shards[0].0.display()
            )));
        }
    }
    let files = test262_files(&root.join("third_party/test262")).map_err(|e| {
        cell(
            Status::Open,
            format!(
                "{} GC-stress shard files, but the split cannot be checked: {e}",
                shards.len()
            ),
        )
    })?;
    // The exact files of each shard JSON (C-G1): a count and the non-passing paths do not show
    // a shard that ran a passing test of another shard in place of one of its own.
    let mut selected = Vec::new();
    for (p, j) in shards {
        selected.push(selected_paths(j).map_err(|e| fail(format!("{}: {e}", p.display())))?);
    }
    // The smallest split in which each JSON selected exactly the files of one shard.
    let mut found = None;
    for n in shards.len()..=MAX_GC_SHARDS {
        let Ok(split) = gc_split(&files, n) else {
            break;
        };
        let members = shard_members(&files, &split);
        let placed: Option<Vec<usize>> = selected
            .iter()
            .map(|sel| members.iter().position(|m| m == sel))
            .collect();
        if let Some(placed) = placed {
            found = Some((n, placed));
            break;
        }
    }
    let Some((n, placed)) = found else {
        return Err(fail(format!(
            "they are not shards of one `--gc-shard <k>/<n>` split of this tree's test262 checkout (n {}..={MAX_GC_SHARDS}): a shard did not run and skip exactly the files of one shard",
            shards.len()
        )));
    };
    let mut seen = BTreeMap::new();
    for (i, k) in placed.iter().enumerate() {
        if let Some(other) = seen.insert(*k, i) {
            return Err(fail(format!(
                "{} and {} are both shard {}/{n}",
                shards[other].0.display(),
                shards[i].0.display(),
                k + 1
            )));
        }
    }
    let missing: Vec<String> = (0..n)
        .filter(|k| !seen.contains_key(k))
        .map(|k| format!("{}/{n}", k + 1))
        .collect();
    if !missing.is_empty() {
        return Err(fail(format!(
            "shard(s) {} of the {n}-way split are missing, so the run does not cover the corpus",
            missing.join(", ")
        )));
    }
    // Every shard once, each with exactly its own files: together they select every test file
    // of the checkout once (the split is a partition; checked here too).
    let union: usize = selected.iter().map(Vec::len).sum();
    if union != files.len() {
        return Err(fail(format!(
            "the shards select {union} files, the checkout has {}",
            files.len()
        )));
    }
    // Merge: the counts add up, the paths join (the shards are disjoint by construction).
    let sum = |key: &[&str], f: &str| -> f64 {
        shards
            .iter()
            .filter_map(|(_, j)| j.path(key)?.get(f)?.num())
            .sum()
    };
    let mut results: Vec<Json> = Vec::new();
    let mut skipped_paths: Vec<Json> = Vec::new();
    let mut runner_new: Vec<Json> = Vec::new();
    let mut runner_now: Vec<Json> = Vec::new();
    for (_, j) in shards {
        let arr = |key: &[&str]| j.path(key).and_then(Json::arr).unwrap_or(&[]).to_vec();
        results.extend(arr(&["results"]));
        skipped_paths.extend(arr(&["skipped_paths"]));
        runner_new.extend(arr(&["expectations", "new_failures"]));
        runner_now.extend(arr(&["expectations", "now_passing"]));
    }
    let first = &shards[0].1;
    let copy = |k: &str| first.get(k).cloned().unwrap_or(Json::Null);
    let counts = |passed: f64, total: f64| {
        Json::Obj(vec![
            ("passed".into(), Json::Num(passed)),
            ("total".into(), Json::Num(total)),
        ])
    };
    let skipped: f64 = shards
        .iter()
        .filter_map(|(_, j)| j.get("skipped").and_then(Json::num))
        .sum();
    let merged = Json::Obj(vec![
        ("command".into(), copy("command")),
        ("target".into(), copy("target")),
        ("timeout_ms".into(), copy("timeout_ms")),
        ("gc_stress".into(), copy("gc_stress")),
        ("feedback_threshold".into(), copy("feedback_threshold")),
        ("skipped".into(), Json::Num(skipped)),
        ("skipped_paths".into(), Json::Arr(skipped_paths)),
        (
            "files".into(),
            counts(sum(&["files"], "passed"), sum(&["files"], "total")),
        ),
        ("results".into(), Json::Arr(results)),
        (
            "expectations".into(),
            Json::Obj(vec![
                (
                    "file".into(),
                    first
                        .path(&["expectations", "file"])
                        .cloned()
                        .unwrap_or(Json::Null),
                ),
                ("new_failures".into(), Json::Arr(runner_new)),
                ("now_passing".into(), Json::Arr(runner_now)),
            ]),
        ),
    ]);
    let names: Vec<String> = shards
        .iter()
        .zip(&placed)
        .map(|((p, _), k)| format!("{} = {}/{n}", p.display(), k + 1))
        .collect();
    Ok((
        format!(
            "{n} shards of one `--gc-shard <k>/{n}` split, each once, merged ({})",
            names.join(", ")
        ),
        merged,
    ))
}
