#!/usr/bin/env python3
"""Story 5.6: the client registers exactly the §5.3 vocabulary of phase5-grammar.py.

Static: the name tables of Source/Nightfall/Bot/EconomyBotPredicates.cpp (PREDICATES minus the Phase 1
builtin `rejected`, and COMMANDS) must equal the grammar's, in the grammar's order; the operation name
table of EconomyStateSubsystem.cpp must equal ECONOMY_OPERATIONS; every Phase 5 command has a console
command and every ItemOperation 14..42 of items.proto a name. The Unreal automation test
Nightfall.Economy.Bot.Phase5ScenarioRegistry parses the 36 files through the same registry.
"""
import importlib.util
from pathlib import Path
import re
import unittest

CLIENT = Path(__file__).resolve().parents[2]
REPO = CLIENT.parents[1]
spec = importlib.util.spec_from_file_location('grammar5', CLIENT / 'Scripts/phase5-grammar.py')
grammar = importlib.util.module_from_spec(spec)
spec.loader.exec_module(grammar)
PREDICATES_CPP = (CLIENT / 'Source/Nightfall/Bot/EconomyBotPredicates.cpp').read_text()
STATE_CPP = (CLIENT / 'Source/Nightfall/Economy/EconomyStateSubsystem.cpp').read_text()
ITEMS_PROTO = (REPO / 'packages/proto/nightfall/v1/items.proto').read_text()


def table(source, name):
    block = re.search(r'const TCHAR\* const ' + name + r'\[\] = \{(.*?)\};', source, re.S)
    assert block, name
    return re.findall(r'TEXT\("([^"]+)"\)', block[1])


class Registry(unittest.TestCase):
    def test_predicate_table_equals_grammar_order(self):
        expected = [p for p in grammar.PREDICATES if p != 'rejected']
        self.assertEqual(table(PREDICATES_CPP, 'Predicates'), expected)
        self.assertEqual(len(expected), 96)

    def test_command_table_equals_grammar_order_and_console_commands(self):
        commands = table(PREDICATES_CPP, 'Commands')
        self.assertEqual(commands, list(grammar.COMMANDS))
        for command in commands:
            self.assertIn(f'"{command}")', PREDICATES_CPP, f'{command} console command')
            self.assertIn(f'TEXT("{command.lower()}")', STATE_CPP, f'{command} operation mapping')

    def test_operation_names_match_grammar_and_items_proto(self):
        block = re.search(r'OperationNames\[\] = \{(.*?)\};', STATE_CPP, re.S)[1]
        names = re.findall(r'TEXT\("([^"]+)"\)', block)
        self.assertEqual(set(names[14:]), set(grammar.ECONOMY_OPERATIONS))
        proto = re.findall(r'ITEM_OPERATION_([A-Z_]+) = (\d+);', ITEMS_PROTO)
        for enum_name, value in proto:
            if int(value) >= 14:
                self.assertEqual(names[int(value)], enum_name.lower(), enum_name)
        self.assertEqual(len(names), 43)

    def test_every_grammar_predicate_has_an_evaluation_branch(self):
        for name in grammar.PREDICATES:
            if name == 'rejected':
                continue
            self.assertIn(f'TEXT("{name}")', PREDICATES_CPP, name)


if __name__ == '__main__':
    unittest.main()
