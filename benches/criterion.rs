//! Criterion benches. Pin perf on the hot paths: parse, stream
//! evaluator, tree evaluator, attribute rewrite.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use mdqy::{parse, Query};
use pulldown_cmark::Parser;

/// Build a synthetic markdown document with `headings` H2 sections
/// and a few code fences and links sprinkled through each.
fn corpus(headings: usize) -> String {
    let mut out = String::with_capacity(headings * 320);
    out.push_str("# Big Corpus\n\nIntro paragraph with a [link](https://example.com).\n\n");
    for i in 0..headings {
        use std::fmt::Write as _;
        let _ = writeln!(out, "## Section {i}\n");
        let _ = writeln!(
            out,
            "Paragraph with an [inline link](http://example.com/{i}).\n"
        );
        let _ = writeln!(
            out,
            "```rust\nfn section_{i}() {{ println!(\"{i}\"); }}\n```\n"
        );
        if i % 3 == 0 {
            let _ = writeln!(out, "| col1 | col2 |\n|------|------|\n| a{i} | b{i} |\n");
        }
    }
    out
}

fn bench_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("parse");
    for &n in &[64usize, 512, 4096] {
        let src = corpus(n);
        group.throughput(Throughput::Bytes(src.len() as u64));
        group.bench_with_input(BenchmarkId::new("headings", n), &src, |b, src| {
            b.iter(|| {
                let events: Vec<_> = Parser::new_ext(src, mdqy::markdown_options()).collect();
                criterion::black_box(events.len())
            });
        });
    }
    group.finish();
}

fn bench_stream_vs_tree(c: &mut Criterion) {
    let src = corpus(1024);

    let mut group = c.benchmark_group("headings_text_end_to_end");
    group.throughput(Throughput::Bytes(src.len() as u64));

    // Stream mode: events → query, one pass, no tree allocation.
    let q_stream = Query::compile("headings | .text").unwrap();
    assert_eq!(q_stream.mode_name(), "stream");
    group.bench_function("stream", |b| {
        b.iter(|| {
            let events = Parser::new_ext(&src, mdqy::markdown_options());
            let n = q_stream.run(events).filter_map(Result::ok).count();
            criterion::black_box(n)
        });
    });

    // Tree mode: parse to Node tree, then run.
    let q_tree = Query::compile("[headings | .text]").unwrap();
    assert_eq!(q_tree.mode_name(), "tree");
    group.bench_function("tree", |b| {
        b.iter(|| {
            let root = parse(&src);
            let n = q_tree.run_tree(&root).filter_map(Result::ok).count();
            criterion::black_box(n)
        });
    });
    group.finish();
}

fn bench_link_rewrite(c: &mut Criterion) {
    let src = corpus(512);
    let expr = r#"(.. | select(type == "link")).href |= sub("http:"; "https:")"#;
    let q = Query::compile(expr).unwrap();

    let mut group = c.benchmark_group("rewrite_links");
    group.throughput(Throughput::Bytes(src.len() as u64));
    group.bench_function("http_to_https", |b| {
        b.iter(|| {
            let out = q.transform_bytes(src.as_bytes()).unwrap();
            criterion::black_box(out.len())
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_parse,
    bench_stream_vs_tree,
    bench_link_rewrite
);
criterion_main!(benches);
