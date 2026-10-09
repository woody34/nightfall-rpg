//! Event-driven E6.1 scenario. Use the isolated data copy prepared by record-fight.sh.
use super::*;
use pb::world_event::Event;

impl Seen {
    fn mark(&self) -> usize {
        self.events.lock().len()
    }

    async fn until(&self, from: usize, pred: impl Fn(&Event) -> bool) -> anyhow::Result<Event> {
        tokio::time::timeout(Duration::from_secs(120), async {
            loop {
                if self.rejected.load(Ordering::Relaxed) > 1 {
                    bail!("unexpected rejected fight intent");
                }
                {
                    let events = self.events.lock();
                    if let Some(e) = events.iter().skip(from).find(|e| pred(e)) {
                        return Ok(e.clone());
                    }
                }
                pause(50).await;
            }
        })
        .await
        .context("timed out waiting for fight event")?
    }
}

async fn walk(tx: &mut Tx, seen: &Seen, id: &str, seq: u32, x: f32, y: f32) -> anyhow::Result<()> {
    let from = seen.mark();
    move_to(tx, seq, x, y).await?;
    seen.until(from, |e| matches!(e, Event::Move(m) if m.entity_id == id && m.speed == 0.0 && m.position == Some(pb::Position { x, y }))).await?;
    Ok(())
}

async fn attack(tx: &mut Tx, seq: u32, target: &str) -> anyhow::Result<()> {
    send(
        tx,
        seq,
        client_message::Intent::SetTarget(pb::SetTargetRequest {
            entity_id: target.into(),
        }),
    )
    .await?;
    send(tx, seq + 1, client_message::Intent::Attack(pb::AttackRequest {})).await
}

pub(super) async fn run(
    url: &str,
    (a_id, a_ticket, sa): (&str, &str, Arc<Seen>),
    (b_id, b_ticket, sb): (&str, &str, Arc<Seen>),
) -> anyhow::Result<()> {
    let mut a = connect(url, a_ticket, sa.clone()).await?;
    let mut b = connect(url, b_ticket, sb.clone()).await?;
    // Preserve movement/rejection coverage before approaching the spawn slots.
    move_to(&mut a, 1, 300.0, 10.0).await?;
    move_to(&mut b, 1, 20.0, 20.0).await?;
    pause(500).await;
    stop(&mut b, 2).await?;
    // MoveTo is limited to 64 tiles: approach in legal waypoints.
    for (seq, x) in [(2, 35.0), (3, 70.0)] {
        tokio::try_join!(
            walk(&mut a, &sa, a_id, seq, x, x),
            walk(&mut b, &sb, b_id, seq + 1, x, x)
        )?;
    }
    // Stay outside aggression radius, but inside AOI and selection range.
    tokio::try_join!(
        walk(&mut a, &sa, a_id, 4, 90.0, 100.0),
        walk(&mut b, &sb, b_id, 5, 90.0, 103.0)
    )?;
    let Event::Spawn(npc) = sa
        .until(0, |e| matches!(e, Event::Spawn(s) if s.template_id == "keltir"))
        .await?
    else {
        bail!("expected spawn event");
    };
    println!("A chases {}", npc.entity_id);
    attack(&mut a, 5, &npc.entity_id).await?;
    sa.until(0, |e| matches!(e, Event::AttackResult(r) if r.attacker == a_id && r.damage > 0))
        .await?;
    // B joins against another clan member, leaving the first kill/level-up to A.
    let Event::Spawn(other) = sb.until(0, |e| matches!(e, Event::Spawn(s) if s.template_id == "keltir" && s.entity_id != npc.entity_id)).await? else { bail!("expected spawn event"); };
    attack(&mut b, 6, &other.entity_id).await?;
    sa.until(
        0,
        |e| matches!(e, Event::EntityDied(d) if d.entity == npc.entity_id && d.killer == a_id),
    )
    .await?;
    sa.until(0, |e| matches!(e, Event::LevelUp(l) if l.entity == a_id))
        .await?;
    println!("A killed a keltir and levelled; retreat to exercise leash");
    send(&mut b, 8, client_message::Intent::StopAttack(pb::StopAttackRequest {})).await?;
    tokio::try_join!(
        walk(&mut a, &sa, a_id, 7, 80.0, 80.0),
        walk(&mut b, &sb, b_id, 9, 80.0, 83.0)
    )?;
    sa.until(0, |e| matches!(e, Event::Despawn(d) if d.entity_id == npc.entity_id))
        .await?;
    let Event::Spawn(reborn) = sa
        .until(
            0,
            |e| matches!(e, Event::Spawn(s) if s.template_id == "keltir" && s.life_incarnation > 1),
        )
        .await?
    else {
        bail!("expected spawn event");
    };
    println!("corpse decayed and keltir respawned: {}", reborn.entity_id);
    // A stands in range without attacking; B disconnects with an active attack.
    tokio::try_join!(
        walk(&mut a, &sa, a_id, 8, 100.0, 100.0),
        walk(&mut b, &sb, b_id, 10, 92.0, 100.0)
    )?;
    let from = sb.mark();
    attack(&mut b, 11, &reborn.entity_id).await?;
    sb.until(from, |e| matches!(e, Event::AttackStarted(s) if s.attacker == b_id))
        .await?;
    b.lock().await.close().await?;
    sa.until(0, |e| matches!(e, Event::EntityDied(d) if d.entity == a_id))
        .await?;
    println!("A died; respawning at safe point");
    send(&mut a, 9, client_message::Intent::Respawn(pb::RespawnRequest {})).await?;
    sa.until(0, |e| matches!(e, Event::EntityRespawned(r) if r.entity == a_id && r.position == Some(pb::Position { x: 126.0, y: 126.0 }))).await?;
    // Real RNG rolls, never forced outcomes: a seed may need more than one life to
    // demonstrate a crit. Stop once hit/miss/crit have all arrived, or fail explicitly.
    for attempt in 0..12 {
        let covered = {
            let events = sa.events.lock();
            [
                pb::AttackOutcome::Miss,
                pb::AttackOutcome::Hit,
                pb::AttackOutcome::Crit,
            ]
            .iter()
            .all(|outcome| {
                events.iter().any(
                    |e| matches!(e, Event::AttackResult(r) if r.outcome == i32::from(*outcome)),
                )
            })
        };
        if covered {
            break;
        }
        if attempt == 11 {
            bail!("seed did not produce hit/miss/crit within 12 lives");
        }
        println!("waiting for all RNG outcomes: another combat life");
        let from = sa.mark();
        let seq = 10 + attempt * 5;
        walk(&mut a, &sa, a_id, seq, 100.0, 100.0).await?;
        // Attack voluntarily cancels the ten-minute spawn protection; stop before
        // impact so this extra life cannot add another kill/level-up.
        attack(&mut a, seq + 1, &reborn.entity_id).await?;
        pause(200).await;
        send(&mut a, seq + 3, client_message::Intent::StopAttack(pb::StopAttackRequest {})).await?;
        sa.until(from, |e| matches!(e, Event::EntityDied(d) if d.entity == a_id))
            .await?;
        send(&mut a, seq + 4, client_message::Intent::Respawn(pb::RespawnRequest {})).await?;
        sa.until(from, |e| matches!(e, Event::EntityRespawned(r) if r.entity == a_id))
            .await?;
    }
    pause(300).await;
    a.lock().await.close().await?;
    pause(300).await;
    println!("fight complete; stop API gracefully and export the epoch");
    Ok(())
}
