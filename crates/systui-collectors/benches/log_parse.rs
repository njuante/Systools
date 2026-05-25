//! Large-journal throughput benchmark for the logs collector.
//!
//! Measures `LogsCollector::collect` (the journalctl JSON parse path) over a
//! `MockTransport` serving journals of growing size, so the "logs stay responsive
//! on multi-hundred-MB journals" claim from phase 10 (S10.4) is measured rather
//! than assumed. Run with `cargo bench -p systui-collectors`.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use systui_collectors::{LogQuery, LogsCollector};
use systui_core::Collector;
use systui_transport::MockTransport;
use tokio::runtime::Runtime;

/// Build `n` lines of realistic `journalctl -o json` output.
fn big_journal(n: usize) -> String {
    let mut s = String::with_capacity(n * 180);
    for i in 0..n {
        s.push_str(&format!(
            "{{\"__REALTIME_TIMESTAMP\":\"{}\",\"PRIORITY\":\"{}\",\"SYSLOG_IDENTIFIER\":\"svc{}\",\"MESSAGE\":\"event {i} on the host with some detail\"}}\n",
            1_700_000_000_000_000u64 + i as u64,
            i % 8,
            i % 50,
        ));
    }
    s
}

fn bench_log_parse(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("logs_collect");

    for &n in &[1_000usize, 50_000, 200_000] {
        let query = LogQuery {
            lines: n,
            ..LogQuery::default()
        };
        let cmd = format!("journalctl -p 3 -n {n} -o json --no-pager");
        let transport = MockTransport::new().with_stdout(cmd, big_journal(n));
        let collector = LogsCollector::with_query(query);

        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| {
                let entries = rt.block_on(collector.collect(&transport)).unwrap();
                black_box(entries.len());
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_log_parse);
criterion_main!(benches);
