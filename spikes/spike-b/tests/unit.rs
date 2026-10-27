//! SP-Unit (M3-P1): the checks of `spike-b unit` (src/unit/checks.rs) at test size, one test
//! per check, on every target `cargo test` runs on. `gate.sh unit` runs them, then the
//! release binary at gate size.

use spike_b::unit::checks::{CHECKS, Scale, on_thread, refusal};

fn run(name: &str) {
    if let Some(r) = refusal() {
        eprintln!("SKIP: {r}");
        return;
    }
    let c = CHECKS
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("no check {name}"));
    match on_thread(c.run, Scale::Test) {
        Ok(lines) => {
            for l in lines {
                println!("    {l}");
            }
        }
        Err(e) => panic!("check ({name}) failed: {}\n{e}", c.title),
    }
}

#[test]
fn a_units_build_with_0_relocations() {
    run("a");
}

#[test]
fn b_load_runs_unit_text_as_t0_does() {
    run("b");
}

#[test]
fn c_unit_text_reads_its_rw_copy() {
    run("c");
}

#[test]
fn d_two_isolates_on_two_threads_share_the_text() {
    run("d");
}

#[test]
fn e_two_units_in_one_isolate() {
    run("e");
}

#[test]
fn f_unwind_through_unit_frames() {
    run("f");
}

#[test]
fn g_every_mismatch_falls_back() {
    run("g");
}

#[test]
fn every_check_has_a_test() {
    let names: Vec<&str> = CHECKS.iter().map(|c| c.name).collect();
    assert_eq!(names, ["a", "b", "c", "d", "e", "f", "g"]);
}
