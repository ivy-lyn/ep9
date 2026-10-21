#![allow(dead_code)] // m2-stub
//! Spike B driver. P0 skeleton: prints the linked contract versions and exits 0. The gate
//! steps come with the spike code (P2 SPIKE-B; `spikes/spike-b/gate.sh` from P4).

fn main() {
    println!(
        "spike-b: skeleton (M0 month 2 P0); {}",
        spike_b::contracts_line()
    );
}
