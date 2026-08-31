use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use std::hint::black_box;

#[allow(dead_code, unused_imports)]
#[path = "../src/usage_log.rs"]
mod usage_log;

const TURN_CONTEXT: &[u8] =
    br#"{"type":"turn_context","payload":{"model":"gpt-5.6-sol","model_provider":"example"}}"#;
const TOKEN_EVENT: &[u8] = br#"{"timestamp":"2026-08-01T10:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":100,"cached_input_tokens":20,"cache_write_input_tokens":3,"output_tokens":8,"reasoning_output_tokens":2,"total_tokens":108}}}}"#;

fn parse_usage_events(criterion: &mut Criterion) {
    criterion.bench_function("usage_log/1000_token_events", |bencher| {
        bencher.iter_batched(
            || {
                let mut state = usage_log::ParserState::default();
                usage_log::parse_line(TURN_CONTEXT, &mut state).unwrap();
                state
            },
            |mut state| {
                for _ in 0..1000 {
                    black_box(usage_log::parse_line(TOKEN_EVENT, &mut state).unwrap());
                }
            },
            BatchSize::SmallInput,
        );
    });
}

criterion_group!(benches, parse_usage_events);
criterion_main!(benches);
