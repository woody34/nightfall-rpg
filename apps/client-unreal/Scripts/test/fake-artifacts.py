#!/usr/bin/env python3
"""Test fixtures derived from checked-in catalogues, never used by production runners."""
import json
from pathlib import Path
import re
import sys
import uuid
import xml.etree.ElementTree as ET

REPO = Path(__file__).resolve().parents[4]


def contract_fixture(amount=1):
    proto = (REPO / 'packages/proto/nightfall/v1/world.proto').read_text()
    numbers = {}
    for group, oneof in (('intents', 'intent'), ('payloads', 'payload'), ('events', 'event')):
        body = re.search(r'oneof ' + oneof + r'\s*\{([^}]+)\}', proto)[1]
        numbers[group] = {name: int(number) for name, number in
                          re.findall(r'\w+\s+(\w+)\s*=\s*(\d+);', body)}
    body = re.search(r'enum RejectReason\s*\{([^}]+)\}', proto)[1]
    numbers['reasons'] = {name: int(number) for name, number in
                          re.findall(r'(REJECT_REASON_\w+)\s*=\s*(\d+);', body)}
    counts = {group: {name: '0' for name in entries} for group, entries in numbers.items()}
    for group in ('intents', 'events'):
        counts[group][next(iter(counts[group]))] = str(amount)
    for name in ('ack', 'event', 'rejected'):
        counts['payloads'][name] = str(amount)
    counts['reasons']['REJECT_REASON_INVALID'] = str(amount)
    counts['close_codes'] = {name: '0' for name in ('4400', '4408', '4409', '4429')}
    return {'schema_version': 1, 'catalogue_source': 'compiled protobuf descriptors; application close codes from API guidelines',
            'wire_numbers': numbers, 'counts': counts,
            'client_frames': str(amount), 'server_frames': str(3 * amount)}


def transition_fixture(amount=1):
    # The fake mirrors the CLI's schema and catalogue without a second maintained table.
    source = (REPO / 'apps/api/src/bin/replay/coverage.rs').read_text()
    result = {'schema_version': 1}
    for group in ('npc_intentions', 'player_attack_states'):
        match = re.search(group + r': table\(\s*&\[([^]]+)\],\s*&\[([\s\S]+?)\],\s*\)', source)
        states = re.findall(r'"([^"]+)"', match[1])
        reachable = set(re.findall(r'\("([^"]+)", "([^"]+)"\)', match[2]))
        result[group] = [{'from': a, 'to': b, 'reachable': (a, b) in reachable, 'count': 0}
                         for a in states for b in states]
        next(row for row in result[group] if row['reachable'])['count'] = amount
    result.update(life_incarnations={'npc:1->2': amount}, deaths={'player': 0, 'npc': amount},
                  respawns={'player': 0, 'npc': amount}, intent_rejected={'Invalid': amount})
    return result


def bot(out, name, result, report_fail, amount):
    if result != 'noreport':
        suite = ET.Element('testsuite', name=name, tests='1', failures=str(report_fail), errors='0')
        props = ET.SubElement(suite, 'properties')
        ET.SubElement(props, 'property', name='sentinel_unallowed', value='1' if result == 'ensure' else '0')
        ET.SubElement(props, 'property', name='own_entity_id',
                      value=str(uuid.uuid5(uuid.NAMESPACE_URL, 'shared-character' if result == 'sameentity' else name)))
        case = ET.SubElement(suite, 'testcase', classname=name, name='nf.Expect')
        if report_fail:
            kind = {'ensure': 'generalError', 'unknown': 'unknown', 'expectation': 'bot_expectation',
                    'scenario': 'bot_scenario'}.get(result, 'bot_assertion')
            ET.SubElement(case, 'failure', type=kind, message='seeded failure').text = 'seeded failure'
        root = suite
        if result in ('skippedfail', 'skippedinfra'):
            # BotJUnit::Write emits an outer testsuites, executed WaitFor, failed Expect,
            # then later assertions skipped after the first failure.
            suite.set('tests', '5')
            suite.set('skipped', '2')
            case.set('name', 'line 2: nf.WaitFor world.ready')
            case.remove(case.find('failure'))
            ET.SubElement(suite, 'testcase', classname='sim.' + name,
                          name='line 3: nf.WaitFor npc.visible')
            failed = ET.SubElement(suite, 'testcase', classname='sim.' + name,
                                   name='line 4: nf.Expect npc.dead')
            ET.SubElement(failed, 'failure', type='bot_assertion', message='predicate failed').text = 'observed: alive'
            for label in ('line 5: nf.Expect xp.changed', 'line 6: nf.WaitFor npc.respawned'):
                later = ET.SubElement(suite, 'testcase', classname='sim.' + name, name=label)
                ET.SubElement(later, 'skipped', message='not reached after failure')
            if result == 'skippedinfra':
                ET.SubElement(props, 'property', name='sentinel_unallowed', value='1')
            root = ET.Element('testsuites', name='nightfall-sim', tests='5', failures='1', errors='0', skipped='2')
            root.append(suite)
        if result == 'inconsistent':
            suite.set('failures', '1')
        ET.ElementTree(root).write(out / f'{name}.xml', encoding='utf-8', xml_declaration=True)
        if result == 'unreadable':
            (out / f'{name}.xml').write_text('<broken')
    if result != 'nocontract':
        (out / f'{name}.coverage.contract.json').write_text(json.dumps(contract_fixture(amount)))


if __name__ == '__main__':
    bot(Path(sys.argv[1]), sys.argv[2], sys.argv[3], int(sys.argv[4]), int(sys.argv[5]))
