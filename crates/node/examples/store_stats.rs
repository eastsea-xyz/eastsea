//! Where a node's `state.redb` bytes go (roadmap B1).
//!
//! `cargo run -p aether-node --example store_stats -- <copy of state.redb> [--compact]`
//! Run it on a copy: opening a store takes its lock, and `--compact` rewrites it.

use aether_node::store::Store;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: store_stats <state.redb> [--compact]");
    let compact = args.any(|a| a == "--compact");
    let file = std::fs::metadata(&path).expect("store file").len();
    let mut store = Store::open(std::path::Path::new(&path)).expect("open store");
    let height = store.head().expect("head").map(|(h, _)| h).unwrap_or(0).max(1);
    report(&store, file, height);
    if compact {
        while store.compact().expect("compact") {}
        drop(store);
        let file = std::fs::metadata(&path).expect("store file").len();
        println!("\nafter compaction:");
        let store = Store::open(std::path::Path::new(&path)).expect("reopen");
        report(&store, file, height);
    }
}

fn report(store: &Store, file: u64, height: u64) {
    let s = store.stats().expect("stats");
    println!("height {height}, file {file} B = {:.0} B/block", file as f64 / height as f64);
    println!(
        "allocated {} pages x {} B = {} B; stored {} B, metadata {} B, fragmented {} B",
        s.allocated_pages,
        s.page_size,
        s.allocated_pages * s.page_size,
        s.stored,
        s.metadata,
        s.fragmented
    );
    println!("{:<9} {:>9} {:>12} {:>10} {:>11} {:>8} {:>9}", "table", "entries", "stored B", "meta B", "frag B", "pages", "B/block");
    for t in &s.tables {
        let used = t.pages * s.page_size;
        println!(
            "{:<9} {:>9} {:>12} {:>10} {:>11} {:>8} {:>9.1}",
            t.name,
            t.entries,
            t.stored,
            t.metadata,
            t.fragmented,
            t.pages,
            used as f64 / height as f64
        );
    }
}
