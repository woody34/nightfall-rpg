//! V7 phase-0 economy intents, part one (story 3.5): the dispatcher with its frozen receipts
//! and the party protocol of design §1.4–§1.5, §4.4 and §4.6 (pinned `RequestJoinParty`,
//! `RequestAnswerJoinParty`, `RequestWithDrawalParty`, `RequestOustPartyMember`,
//! `RequestChangePartyLeader`, `RequestPartyLootModification`, `AnswerPartyLootModification`
//! and `L2Party`). Every handler checks before it mutates: a rejection changes nothing and
//! consumes no key (D41–D43).

use crate::domain::zone::phase5::{LootVote, Party, PartyId, TxKind, TxRequest, PARTY_SIZE_CAP};
use crate::domain::zone::phase5_command::LootMode;
use crate::domain::zone::phase5_event::{
    EconomyBody, EconomyEvent, EconomyOutcome, EconomyScope, LootVotePhase, PartyMemberView,
};
use crate::domain::zone::{
    EntityId, EntityKind, ItemInstanceId, Phase5Command, RejectReason, Tick, ZoneEvent, ZoneState,
};

/// `REQUEST_TIMEOUT` (15 s, `L2PcInstance.java:354`), the invitation timeout
/// (`L2Party.java:139-151`) and the loot vote (`L2Party.java:88`), all in ticks.
const REQUEST_TICKS: u64 = 150;

/// How a member leaves (`L2Party.messageType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Removal {
    Left,
    Expelled,
    Disconnected,
}

/// A party's leader (index 0); parties always hold 2..=9 members (§1.11).
fn leader(members: &[EntityId]) -> EntityId {
    members
        .first()
        .copied()
        .unwrap_or_else(|| EntityId::from_uuid(uuid::Uuid::nil()))
}

fn event(tick: Tick, entity: EntityId, body: EconomyBody, audience: Vec<EntityId>) -> ZoneEvent {
    ZoneEvent::Economy(Box::new(EconomyEvent {
        tick,
        entity,
        body,
        audience,
    }))
}

impl ZoneState {
    /// Dispatch one client economy intent (design §2.1 phase 0). Kinds whose stories have not
    /// landed refuse `NotYetImplemented`; system jobs are 3.11.
    #[allow(clippy::too_many_arguments)] // The tick-start item state rides along explicitly.
    pub(super) fn phase5_intent(
        &mut self,
        tick: Tick,
        command: &Phase5Command,
        seq: Option<u32>,
        before_items: Option<&crate::domain::zone::Phase4Snapshot>,
        admitted: &std::collections::BTreeMap<EntityId, u64>,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), RejectReason> {
        if self.phase5.is_none() {
            return Err(RejectReason::NotYetImplemented);
        }
        if command.kind().is_system() {
            return Err(RejectReason::NotPermitted);
        }
        let actor = command.entity().ok_or(RejectReason::NotPermitted)?;
        if matches!(command, Phase5Command::VendorBuy { .. }) {
            return self.vendor_buy(tick, actor, command, seq, events);
        }
        if matches!(command, Phase5Command::VendorSell { .. }) {
            return self.vendor_sell(tick, actor, command, seq, events);
        }
        if matches!(command, Phase5Command::VendorRefund { .. }) {
            return self.vendor_refund(tick, actor, command, seq, events);
        }
        if matches!(command, Phase5Command::MultisellChoose { .. }) {
            return self.multisell_choose(tick, actor, command, seq, events);
        }
        if let Phase5Command::PickUpItem { ground, .. } = command {
            return self.pick_up_item(
                tick,
                actor,
                *ground,
                command,
                seq,
                before_items,
                admitted,
                events,
            );
        }
        let party = match command {
            Phase5Command::PartyInvite { target, mode, .. } => {
                self.party_invite(tick, actor, *target, *mode, events)?
            },
            Phase5Command::PartyAnswer { answer, .. } => {
                self.party_answer(tick, actor, *answer, events)?
            },
            Phase5Command::PartyLeave { .. } => self.party_leave(tick, actor, events)?,
            Phase5Command::PartyKick { member, .. } => {
                self.party_kick(tick, actor, *member, events)?
            },
            Phase5Command::PartyChangeLeader { member, .. } => {
                self.party_change_leader(tick, actor, *member, events)?
            },
            Phase5Command::PartyLootChangeRequest { mode, .. } => {
                self.party_loot_request(tick, actor, *mode, events)?
            },
            Phase5Command::PartyLootChangeAnswer { accept, .. } => {
                self.party_loot_answer(tick, actor, *accept, events)?
            },
            // Pickup resolves above. Vendors, multisell, crafting, trade, stores and
            // containers land with stories 3.7–3.11; until then they refuse.
            Phase5Command::PickUpItem { .. }
            | Phase5Command::VendorBuy { .. }
            | Phase5Command::VendorSell { .. }
            | Phase5Command::VendorRefund { .. }
            | Phase5Command::MultisellChoose { .. }
            | Phase5Command::RegisterRecipe { .. }
            | Phase5Command::DeleteRecipe { .. }
            | Phase5Command::CraftItem { .. }
            | Phase5Command::TradeRequest { .. }
            | Phase5Command::TradeAnswer { .. }
            | Phase5Command::TradeAddItem { .. }
            | Phase5Command::TradeConfirm { .. }
            | Phase5Command::TradeCancel { .. }
            | Phase5Command::SetPrivateStoreSell { .. }
            | Phase5Command::SetPrivateStoreBuy { .. }
            | Phase5Command::SetPrivateManufacture { .. }
            | Phase5Command::ClosePrivateStore { .. }
            | Phase5Command::PrivateStoreBuy { .. }
            | Phase5Command::PrivateStoreSell { .. }
            | Phase5Command::PrivateManufactureOrder { .. }
            | Phase5Command::WarehouseDeposit { .. }
            | Phase5Command::WarehouseWithdraw { .. }
            | Phase5Command::FreightSend { .. }
            | Phase5Command::FreightWithdraw { .. }
            | Phase5Command::MailSend { .. }
            | Phase5Command::MailTakeAttachments { .. }
            | Phase5Command::MailReject { .. }
            | Phase5Command::MailCancel { .. }
            | Phase5Command::MailDelete { .. }
            | Phase5Command::ManorBuySeed { .. }
            | Phase5Command::ManorSellCrop { .. }
            | Phase5Command::ItemAuctionBid { .. }
            | Phase5Command::ItemAuctionCancelBid { .. }
            | Phase5Command::MailMarkRead { .. }
            | Phase5Command::DeliverPending { .. }
            | Phase5Command::AuctionSchedule { .. }
            | Phase5Command::EpochReconcile { .. }
            | Phase5Command::MailExpire { .. } => return Err(RejectReason::NotYetImplemented),
        };
        self.economy_receipt(
            tick,
            actor,
            command,
            seq,
            EconomyScope::Party,
            Vec::new(),
            EconomyOutcome::Party {
                party_id: party.map_or(0, |p| p.0),
            },
            events,
        );
        Ok(())
    }

    /// The owner-only frozen `EconomyResult` of an accepted intent (D53; design §13.2).
    #[allow(clippy::too_many_arguments)] // The receipt's fields, explicit.
    pub(super) fn economy_receipt(
        &self,
        tick: Tick,
        actor: EntityId,
        command: &Phase5Command,
        seq: Option<u32>,
        scope: EconomyScope,
        affected: Vec<ItemInstanceId>,
        outcome: EconomyOutcome,
        events: &mut Vec<ZoneEvent>,
    ) {
        let incarnation = self
            .entities
            .get(&actor)
            .and_then(|e| e.combat.as_ref())
            .map_or(1, |c| c.incarnation);
        let inventory_revision = self
            .phase4
            .as_ref()
            .and_then(|items| items.canonical.players.get(&actor))
            .map_or(0, |player| player.inventory_revision);
        let name = serde_json::to_value(scope)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        events.push(event(
            tick,
            actor,
            EconomyBody::EconomyResult {
                incarnation,
                inventory_revision,
                receipt_id: format!("receipt:{name}:{}", command.request_key()),
                operation: scope,
                affected_instance_ids: affected,
                request_key: command.request_key(),
                request_fingerprint: command.fingerprint_hex(),
                accepted_client_seq: seq.map(u64::from),
                outcome,
            },
            Vec::new(),
        ));
    }

    fn admitted_player(&self, id: EntityId) -> bool {
        self.entities
            .get(&id)
            .is_some_and(|e| e.kind == EntityKind::Player)
            && self
                .phase5
                .as_ref()
                .is_some_and(|p| p.canonical.players.contains_key(&id))
    }

    fn party_of(&self, id: EntityId) -> Option<PartyId> {
        self.phase5
            .as_ref()
            .and_then(|p| p.canonical.players.get(&id))
            .and_then(|p| p.party)
    }

    fn party(&self, id: PartyId) -> Option<&Party> {
        self.phase5
            .as_ref()
            .and_then(|p| p.canonical.parties.get(&id))
    }

    /// `isProcessingRequest()`: the player holds an unexpired request as target, or has an
    /// unexpired outstanding request as requester (`L2PcInstance.java:5023-5025`).
    fn processing_request(&self, id: EntityId, tick: Tick) -> bool {
        self.phase5.as_ref().is_some_and(|p| {
            p.canonical
                .requests
                .values()
                .any(|r| r.expires > tick && (r.target == id || r.requester == id))
        })
    }

    /// The party state message for one audience (Q12: a recipient's first one is a baseline).
    fn party_state(
        &self,
        tick: Tick,
        party: &Party,
        baseline: bool,
        audience: Vec<EntityId>,
    ) -> ZoneEvent {
        let level = |id: &EntityId| {
            self.entities
                .get(id)
                .and_then(|e| e.combat.as_ref())
                .map_or(1, |c| c.sheet.level())
        };
        event(
            tick,
            leader(&party.members),
            EconomyBody::PartyStateChanged {
                party_id: party.id.0,
                members: party
                    .members
                    .iter()
                    .map(|id| PartyMemberView {
                        entity: *id,
                        level: level(id),
                    })
                    .collect(),
                mode: party.distribution,
                baseline,
                disbanded: false,
            },
            audience,
        )
    }

    fn party_dispersed(tick: Tick, party: &Party, audience: Vec<EntityId>) -> ZoneEvent {
        event(
            tick,
            leader(&party.members),
            EconomyBody::PartyStateChanged {
                party_id: party.id.0,
                members: Vec::new(),
                mode: party.distribution,
                baseline: false,
                disbanded: true,
            },
            audience,
        )
    }

    /// `PartyInvite` (`RequestJoinParty.java:48-202`), checks in pinned order.
    fn party_invite(
        &mut self,
        tick: Tick,
        actor: EntityId,
        target: EntityId,
        mode: LootMode,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<Option<PartyId>, RejectReason> {
        // 1. The target exists in the zone and is visible to the requester.
        let visible = self
            .known
            .get(&actor)
            .is_some_and(|(_, known)| known.binary_search(&target).is_ok());
        if !self.admitted_player(target) || !visible {
            return Err(RejectReason::UnknownEntity);
        }
        // 2. Not already in a party. 3. Block lists are vacuous before Phase 7. 4. Not self.
        if self.party_of(target).is_some() {
            return Err(RejectReason::PartyAlreadyInParty);
        }
        if target == actor {
            return Err(RejectReason::NotPermitted);
        }
        // 5. In a party: leader, below 9, no unexpired pending invitation.
        let existing = self.party_of(actor);
        if let Some(id) = existing {
            let party = self.party(id).ok_or(RejectReason::InvalidLoad)?;
            if party.members.first() != Some(&actor) {
                return Err(RejectReason::NotPartyLeader);
            }
            if party.members.len() >= PARTY_SIZE_CAP {
                return Err(RejectReason::PartyFull);
            }
            if party.pending_invite.is_some_and(|until| until > tick) {
                return Err(RejectReason::TargetBusy);
            }
        }
        // 6. The target is not processing a request. One outstanding request per requester
        //    (design §1.5): a second one while the first is unexpired is refused too.
        if self.processing_request(target, tick) || self.processing_request(actor, tick) {
            return Err(RejectReason::TargetBusy);
        }
        let expires = Tick(tick.0.saturating_add(REQUEST_TICKS));
        let phase5 = self.phase5.as_mut().ok_or(RejectReason::InvalidLoad)?;
        phase5.canonical.requests.retain(|_, r| r.expires > tick);
        // An existing party keeps its own mode (`:161`).
        let mode = existing
            .and_then(|id| phase5.canonical.parties.get(&id))
            .map_or(mode, |party| party.distribution);
        phase5.canonical.requests.insert(
            target,
            TxRequest {
                requester: actor,
                target,
                kind: TxKind::PartyInvite { mode },
                expires,
            },
        );
        if let Some(party) = existing.and_then(|id| phase5.canonical.parties.get_mut(&id)) {
            party.pending_invite = Some(expires);
        }
        events.push(event(
            tick,
            target,
            EconomyBody::PartyInviteReceived {
                requester: actor,
                mode,
                expires_tick: expires,
            },
            Vec::new(),
        ));
        Ok(existing)
    }

    /// `PartyAnswer` (`RequestAnswerJoinParty.java`): `1` joins, `0`/`-1` deny.
    fn party_answer(
        &mut self,
        tick: Tick,
        actor: EntityId,
        answer: i32,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<Option<PartyId>, RejectReason> {
        let request = self
            .phase5
            .as_ref()
            .and_then(|p| p.canonical.requests.get(&actor))
            .copied()
            .filter(|r| r.expires > tick)
            .ok_or(RejectReason::NoPendingRequest)?;
        let TxKind::PartyInvite { mode } = request.kind else {
            return Err(RejectReason::NoPendingRequest);
        };
        let requester = request.requester;
        let existing = self.party_of(requester);
        if answer == 1 {
            if !self.admitted_player(requester) {
                return Err(RejectReason::UnknownEntity);
            }
            if self.party_of(actor).is_some() {
                return Err(RejectReason::PartyAlreadyInParty);
            }
            if let Some(id) = existing {
                // Re-check the size: a full party refuses and keeps the request (`:63-69`).
                if self
                    .party(id)
                    .is_some_and(|p| p.members.len() >= PARTY_SIZE_CAP)
                {
                    return Err(RejectReason::PartyFull);
                }
            }
        }
        let phase5 = self.phase5.as_mut().ok_or(RejectReason::InvalidLoad)?;
        phase5.canonical.requests.remove(&actor);
        if answer != 1 {
            if let Some(party) = existing.and_then(|id| phase5.canonical.parties.get_mut(&id)) {
                party.pending_invite = None;
            }
            return Ok(existing);
        }
        let (id, created) = if let Some(id) = existing {
            (id, false)
        } else {
            let id = PartyId(phase5.canonical.next_party);
            phase5.canonical.next_party = phase5.canonical.next_party.saturating_add(1);
            phase5.canonical.parties.insert(
                id,
                Party {
                    id,
                    members: vec![requester],
                    distribution: mode,
                    item_last_loot: 0,
                    loot_vote: None,
                    pending_invite: None,
                },
            );
            if let Some(player) = phase5.canonical.players.get_mut(&requester) {
                player.party = Some(id);
            }
            (id, true)
        };
        let party = phase5
            .canonical
            .parties
            .get_mut(&id)
            .ok_or(RejectReason::InvalidLoad)?;
        let before: Vec<EntityId> = party.members.clone();
        party.members.push(actor);
        party.pending_invite = None;
        // A join cancels a pending loot vote (`L2Party.java:254-256`).
        let cancelled = party.loot_vote.take();
        if let Some(player) = phase5.canonical.players.get_mut(&actor) {
            player.party = Some(id);
        }
        let party = self.party(id).cloned().ok_or(RejectReason::InvalidLoad)?;
        if let Some(vote) = cancelled {
            events.push(event(
                tick,
                leader(&party.members),
                EconomyBody::PartyLootVote {
                    party_id: id.0,
                    mode: vote.mode,
                    phase: LootVotePhase::Cancelled,
                    expires_tick: vote.expires,
                },
                before.clone(),
            ));
        }
        if created {
            // Both founding members see the party for the first time: one baseline.
            events.push(self.party_state(tick, &party, true, party.members.clone()));
        } else {
            events.push(self.party_state(tick, &party, true, vec![actor]));
            events.push(self.party_state(tick, &party, false, before));
        }
        Ok(Some(id))
    }

    /// Remove a member (`L2Party.removePartyMember`, `:333-430`): disband when two remain, or
    /// when the leader leaves by Left/Expelled with `LeavePartyLeader = False`; a disconnected
    /// leader is removed and the next member leads (§4.6).
    fn remove_party_member(
        &mut self,
        tick: Tick,
        id: PartyId,
        member: EntityId,
        how: Removal,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), RejectReason> {
        let leave_leader = self
            .phase5
            .as_ref()
            .is_some_and(|p| p.rules.parts().party.party.leave_party_leader);
        let party = self.party(id).cloned().ok_or(RejectReason::InvalidLoad)?;
        let leader = party.members.first() == Some(&member);
        let disband =
            party.members.len() == 2 || (leader && !leave_leader && how != Removal::Disconnected);
        let phase5 = self.phase5.as_mut().ok_or(RejectReason::InvalidLoad)?;
        if disband {
            // Every member removed; the vote is dropped with no broadcast (`:402-425`).
            phase5.canonical.parties.remove(&id);
            for m in &party.members {
                if let Some(player) = phase5.canonical.players.get_mut(m) {
                    player.party = None;
                }
            }
            let audience: Vec<EntityId> = party
                .members
                .iter()
                .copied()
                .filter(|m| how != Removal::Disconnected || *m != member)
                .collect();
            events.push(Self::party_dispersed(tick, &party, audience));
            return Ok(());
        }
        let entry = phase5
            .canonical
            .parties
            .get_mut(&id)
            .ok_or(RejectReason::InvalidLoad)?;
        entry.members.retain(|m| *m != member);
        if let Some(player) = phase5.canonical.players.get_mut(&member) {
            player.party = None;
        }
        let remaining = self.party(id).cloned().ok_or(RejectReason::InvalidLoad)?;
        if how != Removal::Disconnected {
            events.push(Self::party_dispersed(tick, &party, vec![member]));
        }
        events.push(self.party_state(tick, &remaining, false, remaining.members.clone()));
        Ok(())
    }

    fn party_leave(
        &mut self,
        tick: Tick,
        actor: EntityId,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<Option<PartyId>, RejectReason> {
        let id = self.party_of(actor).ok_or(RejectReason::NotPartyMember)?;
        self.remove_party_member(tick, id, actor, Removal::Left, events)?;
        Ok(Some(id))
    }

    fn party_kick(
        &mut self,
        tick: Tick,
        actor: EntityId,
        member: EntityId,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<Option<PartyId>, RejectReason> {
        let id = self.party_of(actor).ok_or(RejectReason::NotPartyMember)?;
        let party = self.party(id).ok_or(RejectReason::InvalidLoad)?;
        if party.members.first() != Some(&actor) {
            return Err(RejectReason::NotPartyLeader);
        }
        if member == actor || !party.members.contains(&member) {
            return Err(RejectReason::NotPartyMember);
        }
        self.remove_party_member(tick, id, member, Removal::Expelled, events)?;
        Ok(Some(id))
    }

    fn party_change_leader(
        &mut self,
        tick: Tick,
        actor: EntityId,
        member: EntityId,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<Option<PartyId>, RejectReason> {
        let id = self.party_of(actor).ok_or(RejectReason::NotPartyMember)?;
        let party = self.party(id).ok_or(RejectReason::InvalidLoad)?;
        if party.members.first() != Some(&actor) {
            return Err(RejectReason::NotPartyLeader);
        }
        let position = party
            .members
            .iter()
            .position(|m| *m == member)
            .filter(|p| *p != 0)
            .ok_or(RejectReason::NotPartyMember)?;
        let phase5 = self.phase5.as_mut().ok_or(RejectReason::InvalidLoad)?;
        let entry = phase5
            .canonical
            .parties
            .get_mut(&id)
            .ok_or(RejectReason::InvalidLoad)?;
        entry.members.swap(0, position);
        let party = self.party(id).cloned().ok_or(RejectReason::InvalidLoad)?;
        events.push(self.party_state(tick, &party, false, party.members.clone()));
        Ok(Some(id))
    }

    fn party_loot_request(
        &mut self,
        tick: Tick,
        actor: EntityId,
        mode: LootMode,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<Option<PartyId>, RejectReason> {
        let id = self.party_of(actor).ok_or(RejectReason::NotPartyMember)?;
        let party = self.party(id).ok_or(RejectReason::InvalidLoad)?;
        if party.members.first() != Some(&actor) {
            return Err(RejectReason::NotPartyLeader);
        }
        // The same mode is a pinned no-op request (`RequestPartyLootModification.java:50`).
        if mode == party.distribution {
            return Err(RejectReason::NotPermitted);
        }
        if party.loot_vote.is_some() {
            return Err(RejectReason::PartyLootVoteInProgress);
        }
        let expires = Tick(tick.0.saturating_add(REQUEST_TICKS));
        let members = party.members.clone();
        let phase5 = self.phase5.as_mut().ok_or(RejectReason::InvalidLoad)?;
        if let Some(entry) = phase5.canonical.parties.get_mut(&id) {
            entry.loot_vote = Some(LootVote {
                mode,
                answers: std::collections::BTreeSet::new(),
                expires,
            });
        }
        events.push(event(
            tick,
            actor,
            EconomyBody::PartyLootVote {
                party_id: id.0,
                mode,
                phase: LootVotePhase::Requested,
                expires_tick: expires,
            },
            members,
        ));
        Ok(Some(id))
    }

    /// `AnswerPartyLootModification` / `L2Party.answerLootChangeRequest` (`:866-884`): an
    /// answer already counted is ignored (the `contains` check runs first); a decline fails
    /// the vote; enough accepts (`≥ members − 1`, the leader may answer) apply it.
    fn party_loot_answer(
        &mut self,
        tick: Tick,
        actor: EntityId,
        accept: bool,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<Option<PartyId>, RejectReason> {
        let id = self.party_of(actor).ok_or(RejectReason::NotPartyMember)?;
        let party = self.party(id).cloned().ok_or(RejectReason::InvalidLoad)?;
        let vote = party
            .loot_vote
            .clone()
            .ok_or(RejectReason::NoPendingRequest)?;
        if vote.answers.contains(&actor) {
            return Ok(Some(id));
        }
        let phase5 = self.phase5.as_mut().ok_or(RejectReason::InvalidLoad)?;
        let entry = phase5
            .canonical
            .parties
            .get_mut(&id)
            .ok_or(RejectReason::InvalidLoad)?;
        let head = leader(&entry.members);
        let finished = if accept {
            let answers = entry.loot_vote.as_mut().map_or(0, |v| {
                v.answers.insert(actor);
                v.answers.len()
            });
            (answers >= entry.members.len().saturating_sub(1)).then_some(true)
        } else {
            Some(false)
        };
        let Some(applied) = finished else {
            return Ok(Some(id));
        };
        entry.loot_vote = None;
        if applied {
            entry.distribution = vote.mode;
        }
        let party = self.party(id).cloned().ok_or(RejectReason::InvalidLoad)?;
        events.push(event(
            tick,
            head,
            EconomyBody::PartyLootVote {
                party_id: id.0,
                mode: vote.mode,
                phase: if applied {
                    LootVotePhase::Applied
                } else {
                    LootVotePhase::Cancelled
                },
                expires_tick: vote.expires,
            },
            party.members.clone(),
        ));
        if applied {
            events.push(self.party_state(tick, &party, false, party.members.clone()));
        }
        Ok(Some(id))
    }

    /// Phase-2 expiry (c)–(e) of design §2.1: expired requests are dropped (no event), expired
    /// loot votes finish as declined (`PartyLootVote{cancelled}`), expired invitations clear.
    pub(super) fn expire_phase5_requests(&mut self, tick: Tick, events: &mut Vec<ZoneEvent>) {
        let Some(phase5) = &mut self.phase5 else {
            return;
        };
        phase5.canonical.requests.retain(|_, r| r.expires > tick);
        let mut finished = Vec::new();
        for party in phase5.canonical.parties.values_mut() {
            if party.loot_vote.as_ref().is_some_and(|v| v.expires <= tick) {
                if let Some(vote) = party.loot_vote.take() {
                    finished.push((party.members.clone(), party.id, vote));
                }
            }
            if party.pending_invite.is_some_and(|until| until <= tick) {
                party.pending_invite = None;
            }
        }
        for (members, id, vote) in finished {
            events.push(event(
                tick,
                leader(&members),
                EconomyBody::PartyLootVote {
                    party_id: id.0,
                    mode: vote.mode,
                    phase: LootVotePhase::Cancelled,
                    expires_tick: vote.expires,
                },
                members,
            ));
        }
    }

    /// Departure (design §4.6, §9.4): the player leaves its party as `Disconnected`, its
    /// requests (either side) are dropped, then its V7 record goes. Trade and store closure
    /// arrive with 3.9; the refund tab with 3.7.
    pub(super) fn depart_phase5(
        &mut self,
        tick: Tick,
        entity: EntityId,
        events: &mut Vec<ZoneEvent>,
    ) {
        if self.phase5.is_none() {
            return;
        }
        if let Some(id) = self.party_of(entity) {
            // An invariant-violating party cannot block a departure; the record goes anyway.
            let _ = self.remove_party_member(tick, id, entity, Removal::Disconnected, events);
        }
        if let Some(phase5) = &mut self.phase5 {
            phase5
                .canonical
                .requests
                .retain(|_, r| r.requester != entity && r.target != entity);
            phase5.canonical.players.remove(&entity);
        }
    }

    /// The §13.1 recipients of one economy event among `observers` (session players):
    /// the explicit audience, else the owner/subject, else the subject's AOI.
    pub(super) fn economy_recipients(
        event: &EconomyEvent,
        sees: impl Fn(EntityId, EntityId) -> bool,
        observers: impl Iterator<Item = EntityId>,
    ) -> Vec<EntityId> {
        if !event.audience.is_empty() {
            return event.audience.clone();
        }
        match &event.body {
            EconomyBody::EconomyResult { .. }
            | EconomyBody::PartyInviteReceived { .. }
            | EconomyBody::TradeRequestReceived { .. }
            | EconomyBody::TradeSessionChanged { .. }
            | EconomyBody::RecipeBookSync { .. } => vec![event.entity],
            EconomyBody::CorpseStateChanged { .. }
            | EconomyBody::PrivateStoreChanged { .. }
            | EconomyBody::ItemAuctionStatusChanged { .. } => {
                observers.filter(|o| sees(*o, event.entity)).collect()
            },
            // Party-scoped bodies always carry their audience; without one nobody hears them.
            EconomyBody::PartyStateChanged { .. }
            | EconomyBody::PartyLootVote { .. }
            | EconomyBody::LootDistributed { .. } => Vec::new(),
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::too_many_lines,
    clippy::wildcard_enum_match_arm,
    clippy::needless_pass_by_value
)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::domain::zone::phase5::Phase5Player;
    use crate::domain::zone::{
        CellCoord, ItemInstanceRange, PlayerLoad, SessionGeneration, Speed, Vec2Fixed, ZoneBounds,
        ZoneId, ZoneSeed,
    };
    use crate::infrastructure::{class_data, economy_data, item_data, rules_data};

    fn id(n: u128) -> EntityId {
        EntityId::from_uuid(uuid::Uuid::from_u128(n))
    }

    /// A V7 zone with `n` admitted players (V6 + V7 records) who all see each other.
    fn zone(n: u128) -> ZoneState {
        let bundle = item_data::embedded_phase4_bundle().unwrap();
        let mut zone = ZoneState::new_phase5(
            ZoneSeed {
                zone: ZoneId(75),
                epoch: 5,
            },
            ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256)).unwrap(),
            0,
            rules_data::load_rules(&rules_data::RulesSource::embedded())
                .unwrap()
                .rules,
            class_data::load_classes(&class_data::ClassSource::embedded())
                .unwrap()
                .registry,
            (*bundle.skills).clone(),
            bundle.items.clone(),
            ItemInstanceRange::new(1, 1, 10_000).unwrap(),
            economy_data::embedded_phase5_rules().unwrap(),
        )
        .unwrap();
        let stats = zone.rules.clone().unwrap();
        let pos = Vec2Fixed::from_tiles(10, 10);
        for k in 1..=n {
            let combat =
                super::super::player_combat(&stats, &PlayerLoad::fresh("human_mystic")).unwrap();
            zone.spawn(
                zone.next_tick,
                super::super::Spawned {
                    id: id(k),
                    kind: EntityKind::Player,
                    name: "P",
                    pos,
                    speed: Speed::from_milli_tiles_per_tick(0),
                    generation: SessionGeneration(1),
                    combat: Some(combat),
                },
            )
            .unwrap();
            zone.phase4
                .as_mut()
                .unwrap()
                .canonical
                .players
                .insert(id(k), super::super::super::item_functions::tests::player());
            zone.phase5
                .as_mut()
                .unwrap()
                .canonical
                .players
                .insert(id(k), Phase5Player::default());
        }
        let everyone: Arc<[EntityId]> = (1..=n).map(id).collect::<Vec<_>>().into();
        for k in 1..=n {
            zone.known
                .insert(id(k), (CellCoord::of(pos), Arc::clone(&everyone)));
        }
        zone
    }

    fn command(kind: &str, actor: u128, key: u128, extra: serde_json::Value) -> Phase5Command {
        let mut value = serde_json::json!({
            "kind": kind,
            "entity": id(actor),
            "request_key": uuid::Uuid::from_u128(key),
        });
        for (k, v) in extra.as_object().unwrap() {
            value[k] = v.clone();
        }
        serde_json::from_value(value).unwrap()
    }

    fn run(zone: &mut ZoneState, c: &Phase5Command) -> Result<Vec<ZoneEvent>, RejectReason> {
        let mut events = Vec::new();
        let before = zone.phase5.as_ref().unwrap().canonical.clone();
        let result = zone.phase5_intent(
            zone.next_tick,
            c,
            Some(7),
            None,
            &std::collections::BTreeMap::new(),
            &mut events,
        );
        if result.is_err() {
            assert_eq!(
                zone.phase5.as_ref().unwrap().canonical,
                before,
                "a rejection changes nothing"
            );
            assert!(events.is_empty());
        }
        zone.validate_phase5_block().unwrap();
        result.map(|()| events)
    }

    fn bodies(events: &[ZoneEvent]) -> Vec<(&EconomyBody, &[EntityId])> {
        events
            .iter()
            .filter_map(|e| match e {
                ZoneEvent::Economy(ev) => Some((&ev.body, ev.audience.as_slice())),
                _ => None,
            })
            .collect()
    }

    fn invite(
        zone: &mut ZoneState,
        from: u128,
        to: u128,
        key: u128,
    ) -> Result<Vec<ZoneEvent>, RejectReason> {
        run(
            zone,
            &command(
                "party_invite",
                from,
                key,
                serde_json::json!({"target": id(to), "mode": "random"}),
            ),
        )
    }

    fn answer(
        zone: &mut ZoneState,
        who: u128,
        value: i32,
        key: u128,
    ) -> Result<Vec<ZoneEvent>, RejectReason> {
        run(zone, &command("party_answer", who, key, serde_json::json!({"answer": value})))
    }

    fn party(zone: &ZoneState) -> Party {
        zone.phase5
            .as_ref()
            .unwrap()
            .canonical
            .parties
            .values()
            .next()
            .unwrap()
            .clone()
    }

    #[test]
    fn invite_answer_join_and_q12_baselines() {
        let mut zone = zone(4);
        let events = invite(&mut zone, 1, 2, 10).unwrap();
        let b = bodies(&events);
        assert!(matches!(
            b[0].0,
            EconomyBody::PartyInviteReceived {
                mode: LootMode::Random,
                ..
            }
        ));
        assert!(matches!(
            b[1].0,
            EconomyBody::EconomyResult {
                operation: EconomyScope::Party,
                ..
            }
        ));
        let events = answer(&mut zone, 2, 1, 11).unwrap();
        let p = party(&zone);
        assert_eq!(p.members, vec![id(1), id(2)]);
        assert_eq!(p.distribution, LootMode::Random);
        assert_eq!(zone.phase5.as_ref().unwrap().canonical.next_party, 2);
        assert!(zone.phase5.as_ref().unwrap().canonical.requests.is_empty());
        // Party creation: one baseline to both founders.
        let b = bodies(&events);
        assert!(
            matches!(b[0], (EconomyBody::PartyStateChanged { baseline: true, .. }, a) if a == [id(1), id(2)])
        );
        // A third member: baseline to the joiner, delta to the existing members.
        invite(&mut zone, 1, 3, 12).unwrap();
        let events = answer(&mut zone, 3, 1, 13).unwrap();
        let b = bodies(&events);
        assert!(
            matches!(b[0], (EconomyBody::PartyStateChanged { baseline: true, .. }, a) if a == [id(3)])
        );
        assert!(
            matches!(b[1], (EconomyBody::PartyStateChanged { baseline: false, .. }, a) if a == [id(1), id(2)])
        );
        // The existing party keeps its mode even if a new invite names another one.
        run(
            &mut zone,
            &command(
                "party_invite",
                1,
                14,
                serde_json::json!({"target": id(4), "mode": "by_turn"}),
            ),
        )
        .unwrap();
        let request = zone.phase5.as_ref().unwrap().canonical.requests[&id(4)];
        assert_eq!(
            request.kind,
            TxKind::PartyInvite {
                mode: LootMode::Random
            }
        );
    }

    #[test]
    fn invite_checks_run_in_pinned_order() {
        let mut zone = zone(4);
        assert_eq!(invite(&mut zone, 1, 99, 1).unwrap_err(), RejectReason::UnknownEntity);
        assert_eq!(invite(&mut zone, 1, 1, 1).unwrap_err(), RejectReason::NotPermitted);
        invite(&mut zone, 1, 2, 2).unwrap();
        // The target is processing a request; so is the requester.
        assert_eq!(invite(&mut zone, 3, 2, 3).unwrap_err(), RejectReason::TargetBusy);
        assert_eq!(invite(&mut zone, 1, 3, 4).unwrap_err(), RejectReason::TargetBusy);
        answer(&mut zone, 2, 1, 5).unwrap();
        assert_eq!(invite(&mut zone, 3, 2, 6).unwrap_err(), RejectReason::PartyAlreadyInParty);
        assert_eq!(invite(&mut zone, 2, 3, 7).unwrap_err(), RejectReason::NotPartyLeader);
        invite(&mut zone, 1, 3, 8).unwrap();
        // A pending invitation blocks another one from the same party.
        zone.phase5.as_mut().unwrap().canonical.requests.clear();
        assert_eq!(invite(&mut zone, 1, 4, 9).unwrap_err(), RejectReason::TargetBusy);
        // Answers: deny clears request and invitation; an expired request is NoPendingRequest.
        assert_eq!(answer(&mut zone, 3, 1, 10).unwrap_err(), RejectReason::NoPendingRequest);
    }

    #[test]
    fn answers_deny_expire_and_full_parties() {
        let mut zone = zone(3);
        invite(&mut zone, 1, 2, 1).unwrap();
        answer(&mut zone, 2, 0, 2).unwrap();
        assert!(zone.phase5.as_ref().unwrap().canonical.parties.is_empty());
        assert!(zone.phase5.as_ref().unwrap().canonical.requests.is_empty());
        invite(&mut zone, 1, 2, 3).unwrap();
        zone.next_tick = Tick(zone.next_tick.0 + REQUEST_TICKS);
        assert_eq!(answer(&mut zone, 2, 1, 4).unwrap_err(), RejectReason::NoPendingRequest);
        let mut out = Vec::new();
        zone.expire_phase5_requests(zone.next_tick, &mut out);
        assert!(zone.phase5.as_ref().unwrap().canonical.requests.is_empty());
    }

    #[test]
    fn leaving_kicking_disbanding_and_leader_changes() {
        let mut zone = zone(4);
        for (k, key) in [(2, 1), (3, 3), (4, 5)] {
            invite(&mut zone, 1, k, key).unwrap();
            answer(&mut zone, k, 1, key + 1).unwrap();
        }
        assert_eq!(party(&zone).members, vec![id(1), id(2), id(3), id(4)]);
        // A member leaves: removed and shifted; it hears "dispersed", the rest a delta.
        let events = run(&mut zone, &command("party_leave", 3, 20, serde_json::json!({}))).unwrap();
        assert_eq!(party(&zone).members, vec![id(1), id(2), id(4)]);
        let b = bodies(&events);
        assert!(
            matches!(b[0], (EconomyBody::PartyStateChanged { disbanded: true, .. }, a) if a == [id(3)])
        );
        // Kick: leader only, members only.
        assert_eq!(
            run(&mut zone, &command("party_kick", 2, 21, serde_json::json!({"member": id(4)})))
                .unwrap_err(),
            RejectReason::NotPartyLeader
        );
        assert_eq!(
            run(&mut zone, &command("party_kick", 1, 22, serde_json::json!({"member": id(3)})))
                .unwrap_err(),
            RejectReason::NotPartyMember
        );
        // Change leader: swap positions 0 and p.
        run(
            &mut zone,
            &command("party_change_leader", 1, 23, serde_json::json!({"member": id(4)})),
        )
        .unwrap();
        assert_eq!(party(&zone).members, vec![id(4), id(2), id(1)]);
        // A disconnecting leader is removed and the next member leads (§4.6).
        let mut events = Vec::new();
        zone.depart_phase5(zone.next_tick, id(4), &mut events);
        zone.phase4
            .as_mut()
            .unwrap()
            .canonical
            .players
            .remove(&id(4));
        zone.validate_phase5_block().unwrap();
        assert_eq!(party(&zone).members, vec![id(2), id(1)]);
        // A leader leaving by Left with LeavePartyLeader = False disbands; so does len == 2.
        let events = run(&mut zone, &command("party_leave", 2, 24, serde_json::json!({}))).unwrap();
        assert!(zone.phase5.as_ref().unwrap().canonical.parties.is_empty());
        assert!(zone
            .phase5
            .as_ref()
            .unwrap()
            .canonical
            .players
            .values()
            .all(|p| p.party.is_none()));
        let b = bodies(&events);
        assert!(
            matches!(b[0], (EconomyBody::PartyStateChanged { disbanded: true, .. }, a) if a == [id(2), id(1)])
        );
    }

    #[test]
    fn loot_votes_apply_fail_ignore_repeats_and_time_out() {
        let mut zone = zone(3);
        for (k, key) in [(2, 1), (3, 3)] {
            invite(&mut zone, 1, k, key).unwrap();
            answer(&mut zone, k, 1, key + 1).unwrap();
        }
        let vote = |zone: &mut ZoneState, mode: &str, key| {
            run(
                zone,
                &command("party_loot_change_request", 1, key, serde_json::json!({"mode": mode})),
            )
        };
        let reply = |zone: &mut ZoneState, who, accept: bool, key| {
            run(
                zone,
                &command(
                    "party_loot_change_answer",
                    who,
                    key,
                    serde_json::json!({"accept": accept}),
                ),
            )
        };
        assert_eq!(
            vote(&mut zone, "random", 10).unwrap_err(),
            RejectReason::NotPermitted,
            "same mode"
        );
        vote(&mut zone, "by_turn", 11).unwrap();
        assert_eq!(
            vote(&mut zone, "finders_keepers", 12).unwrap_err(),
            RejectReason::PartyLootVoteInProgress
        );
        // One accept of three members is short of members − 1; a repeat is ignored.
        reply(&mut zone, 2, true, 13).unwrap();
        reply(&mut zone, 2, false, 14).unwrap();
        assert!(party(&zone).loot_vote.is_some(), "a counted member's later decline is ignored");
        let events = reply(&mut zone, 3, true, 15).unwrap();
        assert_eq!(party(&zone).distribution, LootMode::ByTurn);
        assert!(bodies(&events).iter().any(|(b, _)| matches!(
            b,
            EconomyBody::PartyLootVote {
                phase: LootVotePhase::Applied,
                ..
            }
        )));
        // A decline fails the vote; a timeout cancels it in phase 2.
        vote(&mut zone, "random", 16).unwrap();
        reply(&mut zone, 3, false, 17).unwrap();
        assert!(party(&zone).loot_vote.is_none());
        assert_eq!(party(&zone).distribution, LootMode::ByTurn);
        vote(&mut zone, "random", 18).unwrap();
        let mut out = Vec::new();
        zone.expire_phase5_requests(Tick(zone.next_tick.0 + REQUEST_TICKS), &mut out);
        assert!(party(&zone).loot_vote.is_none());
        assert!(matches!(
            bodies(&out)[0].0,
            EconomyBody::PartyLootVote {
                phase: LootVotePhase::Cancelled,
                ..
            }
        ));
    }

    #[test]
    fn recipients_follow_the_audience_owner_or_aoi() {
        let ev = |body, audience| EconomyEvent {
            tick: Tick(1),
            entity: id(1),
            body,
            audience,
        };
        let sees = |o: EntityId, s: EntityId| o == s || o == id(2);
        let observers = || [id(1), id(2), id(3)].into_iter();
        let corpse = EconomyBody::CorpseStateChanged {
            spoiled: true,
            seeded: false,
            sweepable: true,
            swept: false,
        };
        assert_eq!(
            ZoneState::economy_recipients(&ev(corpse, Vec::new()), sees, observers()),
            vec![id(1), id(2)]
        );
        let invite = EconomyBody::PartyInviteReceived {
            requester: id(2),
            mode: LootMode::Random,
            expires_tick: Tick(9),
        };
        assert_eq!(
            ZoneState::economy_recipients(&ev(invite, Vec::new()), sees, observers()),
            vec![id(1)]
        );
        let vote = EconomyBody::PartyLootVote {
            party_id: 1,
            mode: LootMode::Random,
            phase: LootVotePhase::Requested,
            expires_tick: Tick(9),
        };
        assert!(ZoneState::economy_recipients(&ev(vote.clone(), Vec::new()), sees, observers())
            .is_empty());
        assert_eq!(
            ZoneState::economy_recipients(&ev(vote, vec![id(3)]), sees, observers()),
            vec![id(3)]
        );
    }
}
