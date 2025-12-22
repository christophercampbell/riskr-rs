use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rust_decimal::Decimal;
use std::collections::HashSet;
use std::sync::Arc;

use riskr::actor::pool::ActorPool;
use riskr::actor::state::UserState;
use riskr::domain::event::{Asset, Chain, Direction, EventId, TxEvent, SCHEMA_VERSION};
use riskr::domain::subject::{AccountId, Address, CountryCode, KycTier, Subject, UserId};
use riskr::domain::Decision;
use riskr::rules::inline::{JurisdictionRule, KycCapRule, OfacRule};
use riskr::rules::streaming::{DailyVolumeRule, StructuringRule};
use riskr::rules::{InlineRule, StreamingRule};

fn create_test_event(user_id: &str, usd_value: Decimal) -> TxEvent {
    let now = chrono::Utc::now();
    TxEvent {
        schema_version: SCHEMA_VERSION.to_string(),
        event_id: EventId::new(),
        occurred_at: now,
        observed_at: now,
        subject: Subject {
            user_id: UserId::new(user_id),
            account_id: AccountId::new("A123"),
            addresses: smallvec::smallvec![Address::new("0x1234567890abcdef")],
            geo_iso: CountryCode::new("US"),
            kyc_tier: KycTier::L2,
        },
        chain: Chain::inline(),
        tx_hash: "0xabc123".to_string(),
        direction: Direction::Outbound,
        asset: Asset::new("USDC"),
        amount: "1000000".to_string(),
        usd_value,
        confirmations: 6,
        max_finality_depth: 12,
    }
}

fn bench_ofac_rule(c: &mut Criterion) {
    let mut sanctions = HashSet::new();
    for i in 0..1000 {
        sanctions.insert(format!("0x{:040x}", i));
    }

    let rule = OfacRule::new("R1_OFAC".to_string(), Decision::RejectFatal, sanctions);

    let event = create_test_event("user1", Decimal::new(1000, 0));

    c.bench_function("ofac_rule_evaluate_miss", |b| {
        b.iter(|| rule.evaluate(black_box(&event)))
    });
}

fn bench_jurisdiction_rule(c: &mut Criterion) {
    let mut blocked = HashSet::new();
    blocked.insert("IR".to_string());
    blocked.insert("KP".to_string());
    blocked.insert("CU".to_string());
    blocked.insert("SY".to_string());
    blocked.insert("RU".to_string());

    let rule = JurisdictionRule::new(
        "R2_JURISDICTION".to_string(),
        Decision::RejectFatal,
        blocked,
    );

    let event = create_test_event("user1", Decimal::new(1000, 0));

    c.bench_function("jurisdiction_rule_evaluate_allowed", |b| {
        b.iter(|| rule.evaluate(black_box(&event)))
    });
}

fn bench_kyc_cap_rule(c: &mut Criterion) {
    let mut caps = std::collections::HashMap::new();
    caps.insert(KycTier::L0, Decimal::new(100, 0));
    caps.insert(KycTier::L1, Decimal::new(1000, 0));
    caps.insert(KycTier::L2, Decimal::new(10000, 0));

    let rule = KycCapRule::new("R3_KYC".to_string(), Decision::HoldAuto, caps);

    let event = create_test_event("user1", Decimal::new(5000, 0));

    c.bench_function("kyc_cap_rule_evaluate_within_cap", |b| {
        b.iter(|| rule.evaluate(black_box(&event)))
    });
}

fn bench_daily_volume_rule(c: &mut Criterion) {
    let rule = DailyVolumeRule::new(
        "R4_DAILY".to_string(),
        Decision::HoldAuto,
        Decimal::new(50000, 0),
    );

    let event = create_test_event("user1", Decimal::new(1000, 0));
    let state = UserState::new(1000);

    c.bench_function("daily_volume_rule_evaluate", |b| {
        b.iter(|| rule.evaluate(black_box(&event), black_box(&state)))
    });
}

fn bench_structuring_rule(c: &mut Criterion) {
    let rule = StructuringRule::new(
        "R5_STRUCTURING".to_string(),
        Decision::Review,
        5,
        Decimal::new(2000, 0),
        Decimal::new(3000, 0),
        std::time::Duration::from_secs(3600),
    );

    let event = create_test_event("user1", Decimal::new(2500, 0));
    let state = UserState::new(1000);

    c.bench_function("structuring_rule_evaluate", |b| {
        b.iter(|| rule.evaluate(black_box(&event), black_box(&state)))
    });
}

fn bench_actor_pool_get(c: &mut Criterion) {
    let streaming_rules: Vec<Arc<dyn StreamingRule>> = vec![
        Arc::new(DailyVolumeRule::new(
            "R4_DAILY".to_string(),
            Decision::HoldAuto,
            Decimal::new(50000, 0),
        )),
    ];

    let pool = ActorPool::new(streaming_rules);

    // Pre-populate with some users
    for i in 0..1000 {
        pool.get_or_create(&format!("user{}", i));
    }

    c.bench_function("actor_pool_get_existing", |b| {
        let mut i = 0u32;
        b.iter(|| {
            let user_id = format!("user{}", i % 1000);
            i = i.wrapping_add(1);
            pool.get_or_create(black_box(&user_id))
        })
    });

    c.bench_function("actor_pool_get_new", |b| {
        let mut i = 1000u32;
        b.iter(|| {
            let user_id = format!("newuser{}", i);
            i = i.wrapping_add(1);
            pool.get_or_create(black_box(&user_id))
        })
    });
}

fn bench_full_inline_pipeline(c: &mut Criterion) {
    // Setup all inline rules
    let mut sanctions = HashSet::new();
    sanctions.insert("0xdead".to_string());

    let mut blocked_countries = HashSet::new();
    blocked_countries.insert("IR".to_string());

    let mut caps = std::collections::HashMap::new();
    caps.insert(KycTier::L2, Decimal::new(10000, 0));

    let rules: Vec<Arc<dyn InlineRule>> = vec![
        Arc::new(OfacRule::new(
            "R1_OFAC".to_string(),
            Decision::RejectFatal,
            sanctions,
        )),
        Arc::new(JurisdictionRule::new(
            "R2_JURISDICTION".to_string(),
            Decision::RejectFatal,
            blocked_countries,
        )),
        Arc::new(KycCapRule::new(
            "R3_KYC".to_string(),
            Decision::HoldAuto,
            caps,
        )),
    ];

    let event = create_test_event("user1", Decimal::new(1000, 0));

    c.bench_function("full_inline_pipeline", |b| {
        b.iter(|| {
            let mut decision = Decision::Allow;
            for rule in &rules {
                let result = rule.evaluate(black_box(&event));
                if result.hit && result.decision > decision {
                    decision = result.decision;
                }
            }
            decision
        })
    });
}

criterion_group!(
    benches,
    bench_ofac_rule,
    bench_jurisdiction_rule,
    bench_kyc_cap_rule,
    bench_daily_volume_rule,
    bench_structuring_rule,
    bench_actor_pool_get,
    bench_full_inline_pipeline,
);

criterion_main!(benches);
