use super::*;

#[allow(clippy::too_many_lines)] // One crash matrix shares flag-only gameplay and durable assertions.
async fn flag_crash(head: u64, partial: bool) {
    use nightfall_api::application::replay_log::AppliedTickRecord;
    let pool = common::pg::migrated_pool()
        .await
        .expect("owned DATABASE_URL required");
    let repo = Arc::new(PgCharacterRepository::new(pool.clone()).with_phase3_rules(rules()));
    let a = seed_full_cp_named(repo.clone(), "FlagFirst").await;
    let b = seed_full_cp_named(repo.clone(), "FlagSecond").await;
    // Equal-priority multipliers have order-sensitive Q rounding. Keep installation
    // order opposite effect ordinal order so recovery cannot sort it accidentally.
    for c in [&a, &b] {
        let saved = repo.load_for_admission(c.id).await.unwrap().unwrap();
        let mut retained = saved.phase3.unwrap();
        retained
            .effect_remainders
            .push(retained_effect(&rules(), 82));
        retained.modifier_install_order = vec![
            RetainedModifierSource::Effect { effect_ordinal: 1 },
            RetainedModifierSource::Effect { effect_ordinal: 0 },
        ];
        repo.checkpoint(
            &CharacterCheckpoint {
                character_id: c.id,
                revision_seen: saved.revision,
                level: saved.level,
                xp: saved.xp,
                hp: saved.hp.unwrap(),
                mp: saved.mp.unwrap(),
                alive: true,
                position: Position { x: 20.0, y: 20.0 },
                idempotency: ("save_checkpoint".into(), IdempotencyKey::new()),
                class_state: Some(saved.class_state),
                phase3: Some(retained),
                phase4: None,
            },
            &[],
        )
        .await
        .unwrap();
    }
    let (zone, driver, log) = actor(repo.clone()).await;
    let registry = SessionRegistry::with_characters(repo.clone());
    for c in [&a, &b] {
        registry
            .admit(&zone, SessionId::new(), SessionGeneration(1), &spawn(c))
            .await
            .unwrap();
    }
    driver.step().await.unwrap();
    let baseline = zone.snapshot().await.unwrap();
    log.write_recovery_snapshot(&baseline).await.unwrap();
    drop(driver);
    zone.stopped().await;
    let probe = Arc::new(BatchProbe {
        inner: repo.clone(),
        armed: AtomicBool::new(false),
        failure: std::sync::atomic::AtomicU8::new(1),
        failed_id: a.id,
        lose_ack: AtomicBool::new(false),
        entered: tokio::sync::Semaphore::new(0),
        gates: [a.id, b.id]
            .into_iter()
            .map(|id| (id, tokio::sync::Semaphore::new(1)))
            .collect(),
        requests: parking_lot::Mutex::default(),
    });
    let mut service = CheckpointService::new(
        probe.clone(),
        Arc::new(InMemorySessionAudit::default()),
        Arc::new(Metrics::detached()),
    );
    service.restore(&baseline);
    let mut state = ZoneState::from_snapshot(baseline.clone()).unwrap();
    let counts = || async {
        sqlx::query_as::<_, (i64, i64, i64)>("SELECT (SELECT count(*) FROM idempotency_keys WHERE operation='learn_skill'), (SELECT count(*) FROM character_transfer_receipts), (SELECT count(*) FROM outbox)").fetch_one(&pool).await.unwrap()
    };
    let facts = counts().await;
    let mut before = Vec::new();
    while state.next_tick() <= Tick(head) {
        let tick = state.next_tick().0;
        let mut commands = vec![];
        {
            let (source, target) = (&a, &b);
            let entity = spawn(source).entity;
            if tick == 10 || tick == 20 {
                commands.push(ZoneInput::system(ZoneCommand::SetTarget {
                    entity,
                    target: Some(spawn(target).entity),
                }));
                commands.push(ZoneInput::system(ZoneCommand::Attack { entity }));
            } else if tick == 11 || tick == 21 {
                commands.push(ZoneInput::system(ZoneCommand::StopAttack { entity }));
            }
        }
        let applied = state.run_tick(state.draft(commands)).unwrap();
        assert!(
            !applied
                .events
                .iter()
                .any(|e| matches!(e, ZoneEvent::AttackResult { .. })),
            "stop before impact: hostile starts change only flags"
        );
        log.append_applied(&AppliedTickRecord::from_applied(ZoneId(77), &applied))
            .await
            .unwrap();
        let boundary = state.snapshot();
        if tick == 10 || tick == 20 {
            {
                let c = &a;
                assert_eq!(
                    boundary.phase3.as_ref().unwrap().entities[&spawn(c).entity]
                        .player
                        .as_ref()
                        .unwrap()
                        .pvp
                        .flag_until,
                    Some(Tick(tick + 1200))
                );
            }
        }
        if tick == head {
            for c in [&a, &b] {
                before.push(repo.load_for_admission(c.id).await.unwrap().unwrap());
            }
            if partial {
                probe.armed.store(true, Ordering::Release);
                assert!(service.admitted(&applied, &boundary).await.is_err());
                assert_eq!(probe.requests.lock().len(), 2);
                assert_eq!(repo.load_for_admission(a.id).await.unwrap().unwrap(), before[0]);
                assert_eq!(
                    repo.load_for_admission(b.id)
                        .await
                        .unwrap()
                        .unwrap()
                        .revision,
                    before[1].revision + 1
                );
            }
        } else {
            service.admitted(&applied, &boundary).await.unwrap();
            if tick == 10 || tick == 20 || tick == 1220 {
                let saved = repo.load_for_admission(a.id).await.unwrap().unwrap();
                assert_eq!(
                    saved.phase3.as_ref().unwrap().flag_remaining_ticks,
                    if tick == 1220 { 20 } else { 0 },
                    "flag starts, refresh and expiry wait for cadence"
                );
                if tick < 50 {
                    assert_eq!(
                        saved.revision,
                        baseline
                            .checkpoints
                            .iter()
                            .find(|p| p.entity == spawn(&a).entity)
                            .unwrap()
                            .revision
                    );
                }
            }
        }
    }
    let expected = state.snapshot();
    let mut recovery = CheckpointService::new(
        repo.clone(),
        Arc::new(InMemorySessionAudit::default()),
        Arc::new(Metrics::detached()),
    );
    recovery.recover(log.as_ref(), ZoneId(77), 1).await.unwrap();
    let mut restored = Vec::new();
    for (c, previous) in [&a, &b].into_iter().zip(before) {
        let saved = repo.load_for_admission(c.id).await.unwrap().unwrap();
        let cp = expected
            .phase3
            .as_ref()
            .unwrap()
            .checkpoint(spawn(c).entity, Tick(head))
            .unwrap();
        assert_eq!(
            cp.flag_remaining_ticks,
            if c.id == a.id {
                1220_u64.saturating_sub(head)
            } else {
                0
            }
        );
        assert_eq!(saved.phase3, Some(cp.clone()));
        assert_eq!(saved.revision, previous.revision + 1);
        assert_eq!(
            saved.class_state, previous.class_state,
            "hostility creates no counters, SP, CP or receipts"
        );
        let combat = expected
            .entities
            .iter()
            .find(|e| e.id == spawn(c).entity)
            .unwrap()
            .combat
            .as_ref()
            .unwrap();
        assert_eq!((saved.hp, saved.mp), (Some(combat.hp), Some(combat.mp)));
        if head < 500 {
            assert_eq!(modifier_order_value(&cp), 8);
            let mut swapped = cp.clone();
            swapped.modifier_install_order.reverse();
            assert_eq!(
                modifier_order_value(&swapped),
                9,
                "sorting installation order changes Q rounding"
            );
            assert_eq!(
                cp.modifier_install_order,
                baseline
                    .phase3
                    .as_ref()
                    .unwrap()
                    .checkpoint(spawn(c).entity, Tick(0))
                    .unwrap()
                    .modifier_install_order
            );
        }
        restored.push(saved);
    }
    let keys: i64 = sqlx::query_scalar("SELECT count(*) FROM idempotency_keys")
        .fetch_one(&pool)
        .await
        .unwrap();
    recovery.recover(log.as_ref(), ZoneId(77), 1).await.unwrap();
    assert_eq!(counts().await, facts);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM idempotency_keys")
            .fetch_one(&pool)
            .await
            .unwrap(),
        keys
    );
    for (c, saved) in [&a, &b].into_iter().zip(&restored) {
        assert_eq!(&repo.load_for_admission(c.id).await.unwrap().unwrap(), saved);
    }
    let (fresh, driver, _) = actor(repo.clone()).await;
    let registry = SessionRegistry::with_characters(repo.clone());
    for _ in 0..7 {
        driver.step().await.unwrap();
    }
    for c in [&a, &b] {
        registry
            .admit(&fresh, SessionId::new(), SessionGeneration(2), &spawn(c))
            .await
            .unwrap();
    }
    driver.step().await.unwrap();
    let admitted = fresh.snapshot().await.unwrap();
    for (c, saved) in [&a, &b].into_iter().zip(&restored) {
        assert_eq!(
            repo.load_for_admission(c.id)
                .await
                .unwrap()
                .unwrap()
                .revision,
            saved.revision + 1
        );
        if head < 500 {
            assert_eq!(
                modifier_order_value(
                    &admitted
                        .phase3
                        .as_ref()
                        .unwrap()
                        .checkpoint(spawn(c).entity, Tick(7))
                        .unwrap()
                ),
                8
            );
        }
        assert_eq!(
            admitted.phase3.as_ref().unwrap().entities[&spawn(c).entity]
                .player
                .as_ref()
                .unwrap()
                .pvp
                .flag_until,
            (c.id == a.id && head < 1220).then_some(Tick(7 + 1220_u64.saturating_sub(head)))
        );
    }
    assert_eq!(counts().await, facts);
}

#[tokio::test]
async fn flag_start_refresh_and_expiry_survive_mid_cadence_and_log_ack_crashes() {
    for head in [35, 50, 1225, 1250] {
        Box::pin(flag_crash(head, false)).await;
    }
}

#[tokio::test]
async fn active_and_expired_flags_survive_partial_cadence_save_twice() {
    for head in [50, 1250] {
        Box::pin(flag_crash(head, true)).await;
    }
}

fn modifier_order_value(cp: &Phase3Checkpoint) -> i64 {
    use nightfall_api::domain::zone::stat_functions::{
        compile_modifier, FunctionConditions, StatCalculator,
    };
    let rules = rules();
    let functions = cp
        .modifier_install_order
        .iter()
        .enumerate()
        .flat_map(|(sequence, source)| {
            let effect_ordinal = match source {
                RetainedModifierSource::Effect { effect_ordinal } => Some(*effect_ordinal),
                RetainedModifierSource::Passive { .. } | RetainedModifierSource::DeathPenalty => {
                    None
                },
            }
            .expect("only effect sources in fixture");
            let reference = cp.effect_remainders[usize::try_from(effect_ordinal).unwrap()].skill;
            let skill = rules.skills().get(reference.id).unwrap();
            skill
                .effects
                .iter()
                .enumerate()
                .flat_map(move |(handler, effect)| {
                    effect
                        .stat_children
                        .iter()
                        .enumerate()
                        .filter(|(_, modifier)| modifier.stat == "pDef")
                        .map(move |(ordinal, modifier)| {
                            compile_modifier(
                                modifier,
                                reference.rank,
                                skill.max_level,
                                u64::try_from(sequence).unwrap() + 1,
                                u32::try_from(handler).unwrap(),
                                u32::try_from(ordinal).unwrap() + 1,
                            )
                            .unwrap()
                        })
                })
        });
    StatCalculator::try_new(functions)
        .unwrap()
        .fold(
            "pDef",
            7,
            FunctionConditions {
                weapon_kind: "SWORD",
                can_resurrect: true,
            },
        )
        .unwrap()
}
