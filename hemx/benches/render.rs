//! Generated-view rendering baseline: `cargo bench -p hemx --bench render`.

use hemplate::Hemplate;
use std::hint::black_box;
use std::time::Instant;

const ITERATIONS: u32 = 100_000;
const ROUNDS: usize = 7;

#[derive(Hemplate)]
struct BenchPage {
    title: String,
    body: String,
    items: Vec<String>,
}

fn page() -> BenchPage {
    BenchPage {
        title: String::from("Production readiness"),
        body: "Checked hypermedia without a parallel client application. ".repeat(4),
        items: (0..20).map(|index| format!("item-{index}")).collect(),
    }
}

fn median_ns(mut render: impl FnMut() -> usize) -> u128 {
    for _ in 0..10_000 {
        black_box(render());
    }
    let mut rounds = [0_u128; ROUNDS];
    for round in &mut rounds {
        let start = Instant::now();
        for _ in 0..ITERATIONS {
            black_box(render());
        }
        *round = start.elapsed().as_nanos() / u128::from(ITERATIONS);
    }
    rounds.sort_unstable();
    rounds[ROUNDS / 2]
}

fn main() {
    let view = page();
    let unhinted = median_ns(|| {
        let mut html = String::new();
        view.render_into(&mut html).expect("generated view renders");
        black_box(html.len())
    });
    let hinted = median_ns(|| {
        let mut html = String::with_capacity(view.size_hint());
        view.render_into(&mut html).expect("generated view renders");
        black_box(html.len())
    });
    println!(
        "unhinted_median_ns={unhinted} hinted_median_ns={hinted} size_hint={} html_len={}",
        view.size_hint(),
        view.render().expect("generated view renders").len(),
    );
}
