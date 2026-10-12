use super::*;

#[allow(clippy::too_many_lines)] // Exercise the complete historical-to-live-to-recovery handoff.
async fn startup_after_historical_epoch(phase3: bool) {
    use nightfall_api::application::replay_log::ZoneSnapshotStore;
    use nightfall_api::application::zone_bootstrap::{ZoneBootstrap, ZoneDefinition};
    use nightfall_api::infrastructure::memory::ManualClock;
    use nightfall_api::infrastructure::postgres::PgZoneSnapshotStore;
    let pool = common::pg::migrated_pool()
        .await
        .expect("owned DATABASE_URL required");
    let repo = Arc::new(PgCharacterRepository::new(pool.clone()).with_phase3_rules(rules()));
    let index = Arc::new(PgZoneSnapshotStore::new(pool.clone()));
    let log = Arc::new(InMemoryEventLog::default());
    let def = ZoneDefinition {
        zone: ZoneId(77),
        name: "Recovery".into(),
        bounds: ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256))
            .unwrap(),
        npcs: vec![],
        npc_templates: vec![],
        spawn_slots: vec![],
        safe_point: Vec2Fixed::from_tiles(126, 126),
        config_hash: String::new(),
    };
    let bootstrap = || {
        ZoneBootstrap::new(
            log.clone(),
            Some(index.clone()),
            Arc::new(ManualClock::default()),
            Arc::new(NoReplayMetrics),
        )
        .with_rules(load_rules(&RulesSource::embedded()).unwrap())
        .with_classes(classes(), String::new())
    };
    let seed = ZoneSeed {
        zone: def.zone,
        epoch: 1,
    };
    let stat_rules = load_rules(&RulesSource::embedded()).unwrap().rules;
    let state = if phase3 {
        ZoneState::new_phase3(seed, def.bounds, 0, stat_rules, classes(), (*rules()).clone())
            .unwrap()
    } else {
        ZoneState::new(seed, def.bounds, 0)
            .with_rules(stat_rules)
            .with_classes(classes())
            .unwrap()
            .with_token_policy()
            .unwrap()
    };
    let mut baseline = state.snapshot();
    baseline.meta.checkpoint_internal_cadence = false;
    let state = ZoneState::from_snapshot(baseline.clone()).unwrap();
    let started = start_epoch(log.as_ref(), &baseline).await.unwrap();
    sqlx::query("INSERT INTO zone_epochs(zone_id,epoch) VALUES(77,1)")
        .execute(&pool)
        .await
        .unwrap();
    let gate = DurableTickGate::new(
        &started,
        log.clone(),
        Arc::new(NoReplayMetrics),
        GateConfig::default(),
        CancellationToken::new(),
    );
    let (ticks, driver) = manual_ticks();
    let old = ZoneActor::spawn_gated(state, ticks, gate);
    let old_c = CreateCharacter::new(repo.clone(), classes())
        .execute(CreateCharacterInput {
            idempotency_key: IdempotencyKey::new(),
            account_id: AccountId::from_uuid(Uuid::nil()),
            name: "Historical".into(),
            race: Race::Human,
            base_class_id: Some(ClassId(10)),
            appearance: CharacterAppearance::default(),
        })
        .await
        .unwrap();
    let mut service = CheckpointService::new(
        repo.clone(),
        Arc::new(InMemorySessionAudit::default()),
        Arc::new(Metrics::detached()),
    );
    service.restore(&baseline);
    old.checkpoints
        .install(service.with_durability(log.clone(), index.clone()));
    SessionRegistry::with_characters(repo.clone())
        .admit(&old, SessionId::new(), SessionGeneration(1), &spawn(&old_c))
        .await
        .unwrap();
    for _ in 0..36 {
        driver.step().await.unwrap();
    }
    assert!(
        !old.snapshot()
            .await
            .unwrap()
            .meta
            .checkpoint_internal_cadence
    );
    drop(driver);
    old.stopped().await;
    let mut recovered = CheckpointService::new(
        repo.clone(),
        Arc::new(InMemorySessionAudit::default()),
        Arc::new(Metrics::detached()),
    );
    recovered
        .recover_indexed(log.as_ref(), index.as_ref(), def.zone)
        .await
        .unwrap();
    assert!(index.unresolved(def.zone).await.unwrap().is_empty());
    let (ticks, driver) = manual_ticks();
    let live = bootstrap()
        .with_phase3_rules((*rules()).clone())
        .start(&def, ticks)
        .await
        .unwrap();
    live.handle()
        .checkpoints
        .install(recovered.with_durability(log.clone(), index.clone()));
    driver.step().await.unwrap();
    let initial = live.handle().snapshot().await.unwrap();
    assert_eq!(initial.seed.epoch, 2);
    assert!(initial.meta.checkpoint_internal_cadence);
    assert!(
        log.read_snapshot(def.zone, 2)
            .await
            .unwrap()
            .unwrap()
            .snapshot
            .meta
            .checkpoint_internal_cadence
    );
    let marker: Option<i64> = sqlx::query_scalar(
        "SELECT last_checkpointed_tick FROM zone_epochs WHERE zone_id=77 AND epoch=2",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(marker, None, "idle first tick must not inherit recovery writes");
    let c = seed_full_cp(repo.clone()).await;
    SessionRegistry::with_characters(repo.clone())
        .admit(live.handle(), SessionId::new(), SessionGeneration(1), &spawn(&c))
        .await
        .unwrap();
    driver.step().await.unwrap();
    let admitted = repo.load_for_admission(c.id).await.unwrap().unwrap();
    for _ in 2..=36 {
        driver.step().await.unwrap();
    }
    let boundary = live.handle().snapshot().await.unwrap();
    assert!(boundary.meta.checkpoint_internal_cadence);
    assert!(boundary.checkpoints[0].dirty);
    assert_eq!(repo.load_for_admission(c.id).await.unwrap().unwrap(), admitted);
    drop(driver);
    live.handle().stopped().await;
    let counts = || async {
        sqlx::query_as::<_, (i64, i64, i64)>("SELECT (SELECT count(*) FROM idempotency_keys WHERE operation='learn_skill'), (SELECT count(*) FROM character_transfer_receipts), (SELECT count(*) FROM outbox)").fetch_one(&pool).await.unwrap()
    };
    let facts = counts().await;
    let mut recovery = CheckpointService::new(
        repo.clone(),
        Arc::new(InMemorySessionAudit::default()),
        Arc::new(Metrics::detached()),
    );
    recovery.recover(log.as_ref(), def.zone, 2).await.unwrap();
    let restored = repo.load_for_admission(c.id).await.unwrap().unwrap();
    assert_eq!(restored.revision, admitted.revision + 1);
    assert_eq!(
        restored.phase3,
        Some(
            boundary
                .phase3
                .as_ref()
                .unwrap()
                .checkpoint(spawn(&c).entity, Tick(36))
                .unwrap()
        )
    );
    let combat = boundary
        .entities
        .iter()
        .find(|e| e.id == spawn(&c).entity)
        .unwrap()
        .combat
        .as_ref()
        .unwrap();
    assert_eq!((restored.hp, restored.mp), (Some(combat.hp), Some(combat.mp)));
    assert_eq!(restored.class_state, admitted.class_state);
    let keys: i64 = sqlx::query_scalar("SELECT count(*) FROM idempotency_keys")
        .fetch_one(&pool)
        .await
        .unwrap();
    recovery.recover(log.as_ref(), def.zone, 2).await.unwrap();
    assert_eq!(repo.load_for_admission(c.id).await.unwrap().unwrap(), restored);
    assert_eq!(counts().await, facts);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM idempotency_keys")
            .fetch_one(&pool)
            .await
            .unwrap(),
        keys
    );
}

#[tokio::test]
async fn unfinished_v4_recovery_starts_v5_with_fresh_checkpoint_policy() {
    startup_after_historical_epoch(false).await;
}

#[tokio::test]
async fn historical_v5_recovery_starts_v5_with_fresh_checkpoint_policy() {
    startup_after_historical_epoch(true).await;
}
