#!/usr/bin/env python3
"""Shared E6/E5 DSL contract. Syntax validation is separate from client registration."""
import re
from uuid import UUID

SLOTS = ('under head hair hair2 neck rhand chest lhand rear lear gloves legs feet '
         'rfinger lfinger lbracelet rbracelet deco1 deco2 deco3 deco4 deco5 deco6 cloak belt').split()
COMMANDS = {
    'nf.Equip': '<item> [slot] [key]', 'nf.Unequip': '<slot> [key]',
    'nf.UseItem': '<item> [self|target|entity:<uuid>] [key]',
    'nf.Destroy': '<item> <count> [key]', 'nf.Crystallize': '<item> [key]',
    'nf.Enchant': '<scroll> <item> [support:<item>] [key]',
    'nf.AutoShot': '<template> on|off [key]',
    'nf.Refine': '<item> <stone> [target|entity:<uuid>] [key]',
    'nf.CancelRefine': '<item> [target|entity:<uuid>] [key]',
    'nf.ExchangeSA': '<item> <recipe> [target|entity:<uuid>] [key]',
    'nf.GrantItem': '<template> <count> [key]', 'nf.ItemCatalog': '',
}
PREDICATES = {
    'inventory_has': '<template> <comparison> <count>',
    'equipped': '<slot> ==|!= <template-or-zero>', 'enchant_level': '<slot> <comparison> <level>',
    'weight_pct': '<comparison> <exact-decimal-or-ratio>',
    'overload_level': '<comparison> <level>', 'grade_penalty': 'weapon|armor <comparison> <level>',
    'autoshot_on': '<template>', 'arrows_remaining': '<comparison> <count>',
    'inventory_revision': '<comparison> <revision>', 'item_baseline': '', 'item_catalog_ready': '',
    'item_operation': '== <operation>', 'enchant_outcome': '== <outcome>',
    'enchant_result_consistent': '', 'refined': '<item>',
    'item_rpc_error': '== none|FAILED_PRECONDITION|UNAVAILABLE|PERMISSION_DENIED|INVALID_ARGUMENT',
    'item_receipt_matches_first': '', 'blacksmith_known': '',
}
COMPARISONS = {'==', '!=', '>=', '<=', '>', '<'}
OPERATIONS = {'equip', 'unequip', 'use', 'destroy', 'enchant', 'toggle_auto_shot',
              'refine', 'cancel_refine', 'exchange_sa', 'crystallize', 'grant'}
OUTCOMES = {'success', 'success_no_increment', 'safe_failed', 'blessed_reset', 'destroyed'}


def number(text, maximum=2**64-1, zero=False):
    return bool(re.fullmatch(r'[0-9]+', text)) and int(zero is False) <= int(text) <= maximum


def guid(text):
    try:
        return str(UUID(text)) == text.lower() and UUID(text).int != 0
    except ValueError:
        return False


def item(text):
    if text.startswith('instance:'):
        return number(text[9:], 2**63-1)
    return number(text.removeprefix('template:'), 2**32-1)


def command(tokens):
    """None means outside this contract; False means recognized but malformed."""
    name = next((n for n in COMMANDS if n.lower() == tokens[0].lower()), None)
    if name is None:
        return None
    args = list(tokens[1:])
    if name == 'nf.ItemCatalog': return not args
    if args and guid(args[-1]):
        # Positional UUID target is ambiguous: use entity:<uuid> for explicit targets.
        args.pop()
    target = lambda s: s in ('self', 'target') or s.startswith('entity:') and guid(s[7:])
    if name == 'nf.Equip': return len(args) in (1, 2) and item(args[0]) and (len(args) == 1 or args[1] in SLOTS)
    if name == 'nf.Unequip': return len(args) == 1 and args[0] in SLOTS
    if name == 'nf.UseItem': return len(args) in (1, 2) and item(args[0]) and (len(args) == 1 or target(args[1]))
    if name == 'nf.Destroy': return len(args) == 2 and item(args[0]) and number(args[1], 2**63-1)
    if name == 'nf.Crystallize': return len(args) == 1 and item(args[0])
    if name == 'nf.Enchant': return len(args) in (2, 3) and all(item(a) for a in args[:2]) and (len(args) == 2 or args[2].startswith('support:') and item(args[2][8:]))
    if name == 'nf.AutoShot': return len(args) == 2 and number(args[0], 2**32-1) and args[1] in ('on', 'off')
    if name in ('nf.Refine', 'nf.ExchangeSA'):
        return len(args) in (2, 3) and item(args[0]) and (item(args[1]) if name == 'nf.Refine' else number(args[1])) and (len(args) == 2 or target(args[2]) and args[2] != 'self')
    if name == 'nf.CancelRefine': return len(args) in (1, 2) and item(args[0]) and (len(args) == 1 or target(args[1]) and args[1] != 'self')
    if name == 'nf.GrantItem': return len(args) == 2 and number(args[0], 2**32-1) and number(args[1], 2**63-1)
    return False


def predicate(tokens):
    if not tokens or tokens[0] not in PREDICATES: return None
    name, args = tokens[0], tokens[1:]
    if not PREDICATES[name]: return not args
    if name in ('autoshot_on', 'refined'):
        return len(args) == 1 and (number(args[0], 2**32-1) if name == 'autoshot_on' else item(args[0]))
    if name in ('item_operation', 'enchant_outcome', 'item_rpc_error'):
        values = OPERATIONS if name == 'item_operation' else OUTCOMES if name == 'enchant_outcome' else {'none', 'FAILED_PRECONDITION', 'UNAVAILABLE', 'PERMISSION_DENIED', 'INVALID_ARGUMENT'}
        return len(args) == 2 and args[0] == '==' and args[1] in values
    if name in ('inventory_has', 'equipped', 'enchant_level', 'grade_penalty'):
        if len(args) != 3: return False
        first, op, value = args
        valid = number(first, 2**32-1) if name == 'inventory_has' else first in ('weapon', 'armor') if name == 'grade_penalty' else first in SLOTS
        return valid and op in ({'==', '!='} if name == 'equipped' else COMPARISONS) and number(value, zero=True)
    if len(args) != 2 or args[0] not in COMPARISONS: return False
    if name == 'weight_pct':
        return bool(re.fullmatch(r'[0-9]+(?:\.[0-9]+|/[1-9][0-9]*)?', args[1]))
    return number(args[1], zero=True)
