#!/usr/bin/env python3
import importlib.util
from pathlib import Path
import shlex
import unittest

spec = importlib.util.spec_from_file_location('grammar', Path(__file__).resolve().parents[1] / 'phase4-grammar.py')
grammar = importlib.util.module_from_spec(spec)
spec.loader.exec_module(grammar)


class Grammar(unittest.TestCase):
    def test_commands(self):
        for source in ('nf.Equip 72 rhand', 'nf.Unequip rhand', 'nf.UseItem instance:42 self',
                       'nf.Enchant 951 template:72 support:instance:43', 'nf.AutoShot 5789 on',
                       'nf.Refine 72 8723 target', 'nf.CancelRefine 72', 'nf.ExchangeSA 72 4316442132481',
                       'nf.Crystallize 72', 'nf.Destroy 17 2', 'nf.GrantItem 72 1', 'nf.ItemCatalog'):
            self.assertTrue(grammar.command(shlex.split(source)), source)
            if source != 'nf.ItemCatalog':
                self.assertTrue(grammar.command(shlex.split(source + ' 01970000-0000-7000-8000-000000000040')))
        for source in ('nf.Equip', 'nf.Equip 0', 'nf.Equip instance:9223372036854775808', 'nf.Unequip lrhand',
                       'nf.UseItem 72 corpse', 'nf.Enchant 951 72 invented', 'nf.AutoShot 5789 maybe',
                       'nf.Refine 72 8723 self', 'nf.ExchangeSA 72 zero', 'nf.GrantItem 72 -1'):
            self.assertFalse(grammar.command(shlex.split(source)), source)
        self.assertIsNone(grammar.command(['nf.Invented']))

    def test_predicates(self):
        for source in ('inventory_has 72 >= 1', 'equipped rhand == 0', 'enchant_level rhand == 3',
                       'weight_pct >= 1001/10', 'grade_penalty weapon == 4', 'autoshot_on 5789',
                       'arrows_remaining == 0', 'item_operation == equip', 'enchant_outcome == destroyed',
                       'item_rpc_error == FAILED_PRECONDITION', 'item_baseline', 'refined instance:42'):
            self.assertTrue(grammar.predicate(shlex.split(source)), source)
        for source in ('inventory_has 0 >= 1', 'equipped lrhand == 72', 'weight_pct >= 1/0',
                       'grade_penalty both == 0', 'item_operation == invented', 'autoshot_on 5789 on',
                       'item_rpc_error == UNKNOWN', 'enchant_result_consistent extra'):
            self.assertFalse(grammar.predicate(shlex.split(source)), source)
        self.assertIsNone(grammar.predicate(['invented']))


if __name__ == '__main__': unittest.main()
